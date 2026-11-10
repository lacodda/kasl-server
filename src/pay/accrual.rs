//! Accruals: what the hours earned under the terms in force (ADR 0028).
//!
//! `GET /api/v1/me/accruals` for the person, and
//! `GET /api/v1/users/{id}/accruals` for an administrator.
//!
//! An accrual is derived, never stored, like the norm it reads: a function of
//! the days worked, the norm each date owed and the version of the terms in
//! force on it ([`super::terms::in_force`]). Until a pay period is closed,
//! a late upload or a back-dated version simply changes the answer.
//!
//! Three properties hold the arithmetic, and the tests hold them:
//!
//! * **A date earns the same whichever range asks.** The weekly threshold
//!   needs the earlier days of its week, so a range is read out to whole
//!   Monday-to-Sunday weeks and cut back afterwards; and every line is rounded
//!   to the currency's minor unit on its own, so the days of a month add up to
//!   the month, and the months to the year, to the cent.
//! * **No hour is paid twice for being over.** Where several rules reach one
//!   hour - a long Saturday - it takes the largest multiplier, never their
//!   product; and an hour a daily step or a rest day already paid more for is
//!   not counted again towards the weekly threshold.
//! * **Every line can be checked by hand.** A line is units times a rate times
//!   a multiplier, all three written on it, and its amount is that product
//!   rounded once.
//!
//! What this does not accrue is said, not drawn as zero: a day still open, a
//! date with no terms, a date paid by a salary (accrued from the next version
//! on), a day of leave.

use axum::{
    Json,
    extract::{Path, Query, State},
    response::IntoResponse,
};
use chrono::{Datelike, Duration, NaiveDate};
use iso_currency::Currency;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    overtime::{Ladder, Overtime},
    terms::{self, Basis, RATE_SCALE, Version, in_force},
};
use crate::{
    app::AppState,
    audit,
    calendar::{Calendar, Norm, WorkdayKind},
    error::ApiError,
    login::CurrentUser,
    me::{Range, validate_range},
};

const SECONDS_PER_HOUR: i64 = 3600;

/// A day as the accrual reads it: the date, its kind, and the figure the
/// rest of the server reads for it (`workday_figures`, ADR 0023).
#[derive(Debug, Clone, Copy)]
pub struct Worked {
    pub date: NaiveDate,
    pub kind: WorkdayKind,
    /// `None` while the day is open: it has no total yet, and so earns
    /// nothing yet.
    pub worked_seconds: Option<i64>,
}

/// Which rule made an hour pay more than an ordinary one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    Daily,
    Weekly,
    RestDay,
}

/// A stretch of a day's worked time paying one multiplier, in the order the
/// day was worked: ordinary first, then whatever came past a threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stretch {
    seconds: i64,
    multiplier: Decimal,
    /// `None` for ordinary time.
    rule: Option<Rule>,
}

/// What a line is a line of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LineKind {
    /// Hours at the hourly rate.
    Hours,
    /// The share of a day the day rate pays.
    Day,
    /// Hours past a threshold, at the multiplier.
    Overtime,
    /// Hours past a threshold that the day rate already pays for, earning
    /// only what the multiplier adds to them.
    OvertimePremium,
}

/// One line of a day's accrual: units, times a rate, times a multiplier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub kind: LineKind,
    pub rule: Option<Rule>,
    /// Seconds, for a line of hours; `None` for a share of a day.
    pub seconds: Option<i64>,
    /// The share of a day, to four places; `None` for a line of hours.
    pub share: Option<Decimal>,
    /// Per hour for a line of hours - for a day rate, the hourly equivalent of
    /// the day, to four places - and per day for the share of a day.
    pub rate: Decimal,
    pub multiplier: Decimal,
    /// The product, rounded to the currency's minor unit.
    pub amount: Decimal,
}

/// Where a date stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Priced: the lines say how.
    Accrued,
    /// The day has not ended, so it has no total to price.
    Open,
    /// Leave or illness with nothing worked. The tracker accrues no holiday or
    /// sick pay: the law computes those otherwise, and that is bookkeeping.
    Away,
    /// Worked, but no terms were in force - which is not a rate of zero.
    NoTerms,
    /// Paid by a salary, which is prorated over its month rather than priced
    /// a date at a time. Accrued from the version that brings salaries.
    Salaried,
}

/// One date's accrual.
#[derive(Debug, Clone)]
pub struct DayAccrual {
    pub date: NaiveDate,
    pub kind: WorkdayKind,
    /// A date the calendar asks nothing of: a weekend not moved into the week,
    /// a holiday.
    pub rest_day: bool,
    pub worked_seconds: Option<i64>,
    /// What the date owed, as the norm module says it: zero on a rest day and
    /// on a day of leave.
    pub norm_seconds: i64,
    pub status: Status,
    /// The version in force on the date, when there is one.
    pub terms: Option<TermsRef>,
    pub lines: Vec<Line>,
    /// The sum of the lines, in the version's currency; `None` unless accrued.
    pub amount: Option<Decimal>,
}

/// Which version priced a date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TermsRef {
    pub id: i64,
    pub basis: Basis,
    pub currency: String,
}

/// The rest of what an accrual reads.
pub struct Inputs<'a> {
    pub calendar: &'a Calendar,
    pub norm: Norm,
    pub versions: &'a [Version],
}

/// The Monday of a date's week. Weeks run Monday to Sunday here, as they do
/// on every screen of this server.
pub fn monday_of(date: NaiveDate) -> NaiveDate {
    date - Duration::days(i64::from(date.weekday().num_days_from_monday()))
}

/// Accrues every day given.
///
/// `days` are one person's, and must hold every stored day of each week they
/// touch - a weekly threshold reads the days before the one it prices. The
/// handler reads whole weeks and keeps the dates it was asked for.
pub fn accrue(inputs: &Inputs, days: &[Worked]) -> Vec<DayAccrual> {
    let mut days: Vec<Worked> = days.to_vec();
    days.sort_by_key(|day| day.date);

    let mut accrued = Vec::with_capacity(days.len());
    for week in days.chunk_by(|a, b| monday_of(a.date) == monday_of(b.date)) {
        let monday = monday_of(week[0].date);
        let sunday = monday + Duration::days(6);
        let away: Vec<NaiveDate> = week.iter().filter(|day| !day.kind.owes_the_norm()).map(|day| day.date).collect();
        let week_norm = inputs.norm.for_range(inputs.calendar, monday, sunday, &away);

        // Ordinary seconds worked earlier in the week: what a weekly threshold
        // is measured against. Every day's ordinary time counts, priced or not
        // - hours are hours, whatever terms they fell under.
        let mut ordinary_before = 0;
        for day in week {
            let (accrual, ordinary) = accrue_day(inputs, day, week_norm, ordinary_before);
            ordinary_before += ordinary;
            accrued.push(accrual);
        }
    }
    accrued
}

/// One date, given the week so far. Answers the accrual and the ordinary
/// seconds the date adds to its week.
fn accrue_day(inputs: &Inputs, day: &Worked, week_norm: i64, ordinary_before: i64) -> (DayAccrual, i64) {
    let Norm { standard_hours, work_rate } = inputs.norm;
    let rest_day = inputs.calendar.hours_on(day.date, standard_hours) == Decimal::ZERO;
    let norm_seconds = if day.kind.owes_the_norm() {
        inputs.calendar.norm_seconds(day.date, standard_hours, work_rate)
    } else {
        0
    };
    let version = in_force(inputs.versions, day.date);
    let overtime = version.map(|version| version.overtime).unwrap_or_default();

    let mut accrual = DayAccrual {
        date: day.date,
        kind: day.kind,
        rest_day,
        worked_seconds: day.worked_seconds,
        norm_seconds,
        status: Status::Accrued,
        terms: version.map(|version| TermsRef {
            id: version.id,
            basis: version.basis,
            currency: version.currency.clone(),
        }),
        lines: Vec::new(),
        amount: None,
    };

    let Some(worked) = day.worked_seconds else {
        accrual.status = Status::Open;
        return (accrual, 0);
    };
    let worked = worked.max(0);

    let mut stretches = daily_stretches(worked, norm_seconds, rest_day, &overtime);
    let ordinary = weekly_stretches(&mut stretches, ordinary_before, week_norm, overtime.weekly.as_ref());

    accrual.status = match version {
        _ if worked == 0 && !day.kind.owes_the_norm() => Status::Away,
        None => Status::NoTerms,
        Some(version) if version.basis == Basis::Monthly => Status::Salaried,
        Some(_) => Status::Accrued,
    };
    if let (Status::Accrued, Some(version)) = (accrual.status, version) {
        let minor = minor_unit(&version.currency);
        accrual.lines = match version.basis {
            Basis::Hourly => hourly_lines(&stretches, version.rate, minor),
            Basis::Daily => {
                let day_seconds = day_seconds(norm_seconds, standard_hours, work_rate);
                daily_lines(&stretches, worked, day_seconds, version.rate, minor)
            }
            Basis::Monthly => unreachable!("a salaried date is not priced here"),
        };
        accrual.amount = Some(accrual.lines.iter().map(|line| line.amount).sum());
    }
    (accrual, ordinary)
}

/// Splits a day's worked time by the daily rule and the rest-day rule.
///
/// Ordinary time first, then each step of the daily ladder; on a rest day
/// every stretch pays the larger of its own multiplier and the rest day's.
fn daily_stretches(worked: i64, norm_seconds: i64, rest_day: bool, overtime: &Overtime) -> Vec<Stretch> {
    let mut stretches = Vec::new();
    let mut cuts: Vec<(i64, Decimal, Option<Rule>)> = vec![(0, Decimal::ONE, None)];
    if let Some(ladder) = &overtime.daily {
        for (start, multiplier) in ladder.steps(norm_seconds) {
            cuts.push((start, multiplier, Some(Rule::Daily)));
        }
    }
    for (index, (start, multiplier, rule)) in cuts.iter().enumerate() {
        let end = cuts.get(index + 1).map_or(worked, |next| next.0.min(worked));
        let start = (*start).min(worked);
        if end > start {
            stretches.push(Stretch {
                seconds: end - start,
                multiplier: *multiplier,
                rule: *rule,
            });
        }
    }

    if rest_day && let Some(rest) = overtime.rest_days {
        for stretch in &mut stretches {
            if rest >= stretch.multiplier {
                stretch.multiplier = rest;
                stretch.rule = Some(Rule::RestDay);
            }
        }
    }
    merge(stretches)
}

/// Moves the part of a day's ordinary time past the weekly threshold onto the
/// weekly ladder, and answers the day's ordinary seconds - what it adds to
/// the week, whether or not this version has a weekly rule.
///
/// Only ordinary time counts towards the week: an hour a daily step or a rest
/// day already paid more for is not paid again for being in a long week.
fn weekly_stretches(stretches: &mut Vec<Stretch>, before: i64, week_norm: i64, ladder: Option<&Ladder>) -> i64 {
    let ordinary: i64 = stretches.iter().filter(|stretch| stretch.rule.is_none()).map(|stretch| stretch.seconds).sum();
    let Some(ladder) = ladder else { return ordinary };
    if ordinary == 0 {
        return ordinary;
    }

    // The ordinary stretch is the first, and only, one: daily steps come
    // after it and a rest day leaves none.
    let position = stretches
        .iter()
        .position(|stretch| stretch.rule.is_none())
        .expect("ordinary time was counted above");
    let (start, end) = (before, before + ordinary);
    let mut cuts: Vec<(i64, Decimal, Option<Rule>)> = vec![(i64::MIN, Decimal::ONE, None)];
    for (at, multiplier) in ladder.steps(week_norm) {
        cuts.push((at, multiplier, Some(Rule::Weekly)));
    }
    let mut pieces = Vec::new();
    for (index, (at, multiplier, rule)) in cuts.iter().enumerate() {
        let next = cuts.get(index + 1).map_or(i64::MAX, |next| next.0);
        let (from, to) = (start.max(*at), end.min(next));
        if to > from {
            pieces.push(Stretch {
                seconds: to - from,
                multiplier: *multiplier,
                rule: *rule,
            });
        }
    }
    let mut rebuilt = stretches[..position].to_vec();
    rebuilt.extend(pieces);
    rebuilt.extend_from_slice(&stretches[position + 1..]);
    *stretches = merge(rebuilt);
    ordinary
}

/// Joins neighbouring stretches that pay the same for the same reason.
fn merge(stretches: Vec<Stretch>) -> Vec<Stretch> {
    let mut merged: Vec<Stretch> = Vec::with_capacity(stretches.len());
    for stretch in stretches {
        match merged.last_mut() {
            Some(last) if last.multiplier == stretch.multiplier && last.rule == stretch.rule => last.seconds += stretch.seconds,
            _ => merged.push(stretch),
        }
    }
    merged
}

/// How long the day a day rate pays for is, in seconds.
///
/// The date's norm where it owes one. A rest day or a day of leave owes
/// nothing, and a share of nothing is undefined, so there it is the person's
/// own full day; and for somebody whose share of a full day is zero, the
/// installation's full day - the one figure that is never zero.
fn day_seconds(norm_seconds: i64, standard_hours: Decimal, work_rate: Decimal) -> i64 {
    if norm_seconds > 0 {
        return norm_seconds;
    }
    let own = super::overtime::hours_to_seconds(standard_hours * work_rate);
    if own > 0 { own } else { super::overtime::hours_to_seconds(standard_hours) }
}

/// The places a currency's amounts are rounded to.
fn minor_unit(currency: &str) -> u32 {
    // A version's currency was checked against the list when it was written;
    // a code the list has since dropped keeps the places the rate is stored to.
    Currency::from_code(currency).and_then(Currency::exponent).map_or(RATE_SCALE, u32::from)
}

/// Rounds an amount once, the way a person checking it with a calculator
/// does: half a cent and up goes up.
fn round(amount: Decimal, minor: u32) -> Decimal {
    amount.round_dp_with_strategy(minor, RoundingStrategy::MidpointAwayFromZero)
}

/// `seconds` at `rate` an hour times `multiplier`, rounded.
fn hours_amount(seconds: i64, rate: Decimal, multiplier: Decimal, minor: u32) -> Decimal {
    round(Decimal::from(seconds) * rate * multiplier / Decimal::from(SECONDS_PER_HOUR), minor)
}

fn hourly_lines(stretches: &[Stretch], rate: Decimal, minor: u32) -> Vec<Line> {
    stretches
        .iter()
        .map(|stretch| Line {
            kind: if stretch.rule.is_some() { LineKind::Overtime } else { LineKind::Hours },
            rule: stretch.rule,
            seconds: Some(stretch.seconds),
            share: None,
            rate,
            multiplier: stretch.multiplier,
            amount: hours_amount(stretch.seconds, rate, stretch.multiplier, minor),
        })
        .collect()
}

/// A day rate: the share of the day worked, at most the whole day, and the
/// hours past a threshold on top at the day's hourly equivalent.
///
/// The day pays for its first `day_seconds` of work. An overtime hour inside
/// them is already paid once by the day, so it earns only what its multiplier
/// adds; one past them earns the whole multiplier. Ordinary time past the day
/// earns nothing more - that is what a day rate is.
fn daily_lines(stretches: &[Stretch], worked: i64, day_seconds: i64, rate: Decimal, minor: u32) -> Vec<Line> {
    let mut lines = Vec::new();
    let covered = worked.min(day_seconds);
    if covered > 0 {
        let share = (Decimal::from(covered) / Decimal::from(day_seconds)).round_dp(4).normalize();
        lines.push(Line {
            kind: LineKind::Day,
            rule: None,
            seconds: None,
            share: Some(share),
            rate,
            multiplier: Decimal::ONE,
            amount: round(share * rate, minor),
        });
    }

    let hourly = (rate * Decimal::from(SECONDS_PER_HOUR) / Decimal::from(day_seconds))
        .round_dp(RATE_SCALE)
        .normalize();
    let mut at = 0;
    for stretch in stretches {
        let (start, end) = (at, at + stretch.seconds);
        at = end;
        if stretch.rule.is_none() {
            continue;
        }
        let inside = (end.min(covered) - start).max(0);
        let outside = stretch.seconds - inside;
        if inside > 0 {
            let premium = stretch.multiplier - Decimal::ONE;
            lines.push(Line {
                kind: LineKind::OvertimePremium,
                rule: stretch.rule,
                seconds: Some(inside),
                share: None,
                rate: hourly,
                multiplier: premium,
                amount: hours_amount(inside, hourly, premium, minor),
            });
        }
        if outside > 0 {
            lines.push(Line {
                kind: LineKind::Overtime,
                rule: stretch.rule,
                seconds: Some(outside),
                share: None,
                rate: hourly,
                multiplier: stretch.multiplier,
                amount: hours_amount(outside, hourly, stretch.multiplier, minor),
            });
        }
    }
    lines
}

// The API ---------------------------------------------------------------------

/// A person's accruals over a range.
#[derive(Debug, Serialize)]
pub struct Accruals {
    pub user_id: Uuid,
    pub from: NaiveDate,
    pub to: NaiveDate,
    /// Every stored day in the range, oldest first.
    pub days: Vec<DayView>,
    /// What the accrued dates came to, one entry per currency: amounts in two
    /// currencies are never added (exchange rates are a later version).
    pub totals: Vec<Total>,
    /// What was worked and not priced, and why.
    pub not_accrued: NotAccrued,
}

#[derive(Debug, Serialize)]
pub struct DayView {
    pub date: NaiveDate,
    pub kind: WorkdayKind,
    pub rest_day: bool,
    pub worked_seconds: Option<i64>,
    pub norm_seconds: i64,
    pub status: Status,
    pub terms: Option<TermsRef>,
    pub lines: Vec<LineView>,
    /// A decimal string in the terms' currency; absent unless accrued.
    pub amount: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LineView {
    pub kind: LineKind,
    pub rule: Option<Rule>,
    pub seconds: Option<i64>,
    pub share: Option<String>,
    pub rate: String,
    pub multiplier: String,
    pub amount: String,
}

#[derive(Debug, Serialize)]
pub struct Total {
    pub currency: String,
    pub amount: String,
    /// Seconds worked on the dates accrued in this currency.
    pub worked_seconds: i64,
    /// Of those, the seconds past a threshold.
    pub overtime_seconds: i64,
}

#[derive(Debug, Default, Serialize)]
pub struct NotAccrued {
    /// Seconds worked on dates with no terms in force.
    pub no_terms_seconds: i64,
    /// Seconds worked on dates paid by a salary.
    pub salaried_seconds: i64,
    /// Days still open.
    pub open_days: i64,
}

impl DayView {
    fn from_accrual(accrual: DayAccrual) -> Self {
        let currency = accrual.terms.as_ref().map(|terms| terms.currency.clone()).unwrap_or_default();
        Self {
            date: accrual.date,
            kind: accrual.kind,
            rest_day: accrual.rest_day,
            worked_seconds: accrual.worked_seconds,
            norm_seconds: accrual.norm_seconds,
            status: accrual.status,
            lines: accrual
                .lines
                .iter()
                .map(|line| LineView {
                    kind: line.kind,
                    rule: line.rule,
                    seconds: line.seconds,
                    share: line.share.map(|share| share.normalize().to_string()),
                    rate: terms::written(line.rate, &currency),
                    multiplier: line.multiplier.normalize().to_string(),
                    amount: terms::written(line.amount, &currency),
                })
                .collect(),
            amount: accrual.amount.map(|amount| terms::written(amount, &currency)),
            terms: accrual.terms,
        }
    }
}

/// Builds the answer from the accruals of the dates asked for.
pub fn summarize(user_id: Uuid, range: &Range, accruals: Vec<DayAccrual>) -> Accruals {
    let mut totals: Vec<(String, Decimal, i64, i64)> = Vec::new();
    let mut not_accrued = NotAccrued::default();
    for accrual in &accruals {
        let worked = accrual.worked_seconds.unwrap_or(0);
        match accrual.status {
            Status::Accrued => {
                let terms = accrual.terms.as_ref().expect("an accrued date has terms");
                let overtime: i64 = accrual
                    .lines
                    .iter()
                    .filter(|line| matches!(line.kind, LineKind::Overtime | LineKind::OvertimePremium))
                    .filter_map(|line| line.seconds)
                    .sum();
                let amount = accrual.amount.unwrap_or_default();
                match totals.iter_mut().find(|total| total.0 == terms.currency) {
                    Some(total) => {
                        total.1 += amount;
                        total.2 += worked;
                        total.3 += overtime;
                    }
                    None => totals.push((terms.currency.clone(), amount, worked, overtime)),
                }
            }
            Status::NoTerms => not_accrued.no_terms_seconds += worked,
            Status::Salaried => not_accrued.salaried_seconds += worked,
            Status::Open => not_accrued.open_days += 1,
            Status::Away => {}
        }
    }
    totals.sort_by(|a, b| a.0.cmp(&b.0));

    Accruals {
        user_id,
        from: range.from,
        to: range.to,
        days: accruals.into_iter().map(DayView::from_accrual).collect(),
        totals: totals
            .into_iter()
            .map(|(currency, amount, worked_seconds, overtime_seconds)| Total {
                amount: terms::written(amount, &currency),
                currency,
                worked_seconds,
                overtime_seconds,
            })
            .collect(),
        not_accrued,
    }
}

/// One person's accruals over a range, whoever is asking. Callers check who
/// may read them first.
pub async fn accruals_for(pool: &PgPool, user_id: Uuid, range: &Range) -> Result<Accruals, ApiError> {
    // Whole weeks: a date's weekly overtime depends on the days before it in
    // its week, and a date must earn the same whichever range asks for it.
    let first = monday_of(range.from);
    let last = monday_of(range.to) + Duration::days(6);

    let rows: Vec<(NaiveDate, WorkdayKind, Option<i64>)> = sqlx::query_as(
        "SELECT date, kind, worked_seconds FROM workday_figures
         WHERE user_id = $1 AND date BETWEEN $2 AND $3
         ORDER BY date",
    )
    .bind(user_id)
    .bind(first)
    .bind(last)
    .fetch_all(pool)
    .await?;
    let days: Vec<Worked> = rows
        .into_iter()
        .map(|(date, kind, worked_seconds)| Worked { date, kind, worked_seconds })
        .collect();

    let calendar = Calendar::load(pool, first, last).await?;
    let norm = Norm::load(pool, user_id).await?;
    let versions = terms::versions_of(pool, user_id).await?;

    let accruals = accrue(
        &Inputs {
            calendar: &calendar,
            norm,
            versions: &versions,
        },
        &days,
    )
    .into_iter()
    .filter(|accrual| accrual.date >= range.from && accrual.date <= range.to)
    .collect();
    Ok(summarize(user_id, range, accruals))
}

/// The person's own accruals.
///
/// `/me` like their days and their terms: no role and no department are
/// consulted, so there is no permission here to get wrong.
pub async fn mine(State(state): State<AppState>, user: CurrentUser, Query(range): Query<Range>) -> Result<impl IntoResponse, ApiError> {
    validate_range(&range)?;
    Ok(Json(accruals_for(&state.pool, user.user_id, &range).await?))
}

/// Somebody's accruals. Administrators only, like their terms, until the
/// finance privilege exists; reading another person's is recorded.
pub async fn of_user(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(target): Path<Uuid>,
    Query(range): Query<Range>,
) -> Result<impl IntoResponse, ApiError> {
    user.require_admin()?;
    validate_range(&range)?;
    let email = terms::email_of(&state.pool, target).await?;
    let accruals = accruals_for(&state.pool, target, &range).await?;

    if target != user.user_id {
        // The range, never the amounts: the log is read by administrators,
        // and the finance privilege may separate the two (ADR 0027).
        audit::Entry::new(audit::action::ACCRUALS_VIEWED)
            .by(user.user_id)
            .by_email(&user.email)
            .on(target)
            .labelled(email)
            .with(serde_json::json!({ "from": range.from, "to": range.to }))
            .record(&state.pool)
            .await;
    }

    Ok(Json(accruals))
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};

    use super::*;
    use crate::{
        calendar::{CalendarDay, CalendarDayKind},
        pay::overtime::{Step, Threshold},
    };

    fn date(text: &str) -> NaiveDate {
        text.parse().expect("a test date")
    }

    fn decimal(text: &str) -> Decimal {
        text.parse().expect("a test decimal")
    }

    fn hours(count: i64) -> Option<i64> {
        Some(count * 3600)
    }

    fn version(id: i64, from: &str, basis: Basis, rate: &str, currency: &str, overtime: Overtime) -> Version {
        Version {
            id,
            user_id: Uuid::nil(),
            effective_from: date(from),
            basis,
            rate: decimal(rate),
            currency: currency.to_string(),
            note: None,
            recorded_by: Uuid::nil(),
            recorded_by_name: "admin".to_string(),
            recorded_at: DateTime::<Utc>::UNIX_EPOCH,
            overtime,
        }
    }

    fn full_time() -> Norm {
        Norm {
            standard_hours: Decimal::from(8),
            work_rate: Decimal::ONE,
        }
    }

    fn work(day: &str, worked: Option<i64>) -> Worked {
        Worked {
            date: date(day),
            kind: WorkdayKind::Work,
            worked_seconds: worked,
        }
    }

    /// A ladder from a fixed threshold.
    fn after(hours: &str, multiplier: &str) -> Ladder {
        Ladder {
            after: Threshold::Hours(decimal(hours)),
            multiplier: decimal(multiplier),
            then: None,
        }
    }

    fn then(mut ladder: Ladder, hours: &str, multiplier: &str) -> Ladder {
        ladder.then = Some(Step {
            after: decimal(hours),
            multiplier: decimal(multiplier),
        });
        ladder
    }

    fn past_norm(multiplier: &str) -> Ladder {
        Ladder {
            after: Threshold::Norm,
            multiplier: decimal(multiplier),
            then: None,
        }
    }

    fn run(calendar: &Calendar, norm: Norm, versions: &[Version], days: &[Worked]) -> Vec<DayAccrual> {
        accrue(&Inputs { calendar, norm, versions }, days)
    }

    fn amounts(accruals: &[DayAccrual]) -> Vec<Option<Decimal>> {
        accruals.iter().map(|accrual| accrual.amount).collect()
    }

    /// A line as (kind, rule, seconds, multiplier, amount), for asserting a
    /// day whole.
    type LineRow = (LineKind, Option<Rule>, Option<i64>, Decimal, Decimal);

    fn lines(accrual: &DayAccrual) -> Vec<LineRow> {
        accrual
            .lines
            .iter()
            .map(|line| (line.kind, line.rule, line.seconds, line.multiplier.normalize(), line.amount))
            .collect()
    }

    // 2026-11-09 is a Monday; 2026-11-14 a Saturday.

    #[test]
    fn an_hourly_day_is_its_hours_at_the_rate() {
        let versions = [version(1, "2026-01-01", Basis::Hourly, "25", "USD", Overtime::default())];
        let accruals = run(&Calendar::empty(), full_time(), &versions, &[work("2026-11-09", Some(7 * 3600 + 1800))]);
        assert_eq!(accruals[0].status, Status::Accrued);
        assert_eq!(lines(&accruals[0]), [(LineKind::Hours, None, Some(27_000), Decimal::ONE, decimal("187.50"))]);
        assert_eq!(accruals[0].amount, Some(decimal("187.50")));
    }

    #[test]
    fn a_long_day_pays_its_daily_steps() {
        // California: 1.5 after 8, 2 after 12. Thirteen hours.
        let rules = Overtime {
            daily: Some(then(after("8", "1.5"), "4", "2")),
            ..Overtime::default()
        };
        let versions = [version(1, "2026-01-01", Basis::Hourly, "20", "USD", rules)];
        let accruals = run(&Calendar::empty(), full_time(), &versions, &[work("2026-11-09", hours(13))]);
        assert_eq!(
            lines(&accruals[0]),
            [
                (LineKind::Hours, None, hours(8), Decimal::ONE, decimal("160")),
                (LineKind::Overtime, Some(Rule::Daily), hours(4), decimal("1.5"), decimal("120")),
                (LineKind::Overtime, Some(Rule::Daily), hours(1), decimal("2"), decimal("40")),
            ]
        );
        assert_eq!(accruals[0].amount, Some(decimal("320")));
    }

    #[test]
    fn a_threshold_at_the_norm_moves_with_the_norm() {
        // Russia: the first two hours past the norm at 1.5, then 2. On the
        // eve of a holiday the norm is seven hours, and so is the threshold.
        let rules = Overtime {
            daily: Some(then(past_norm("1.5"), "2", "2")),
            ..Overtime::default()
        };
        let calendar = Calendar::from_days(vec![CalendarDay {
            date: date("2026-11-10"),
            kind: CalendarDayKind::ShortDay,
            note: None,
        }]);
        let versions = [version(1, "2026-01-01", Basis::Hourly, "10", "EUR", rules)];
        let accruals = run(
            &calendar,
            full_time(),
            &versions,
            &[work("2026-11-09", hours(10)), work("2026-11-10", hours(10))],
        );
        let overtime = |accrual: &DayAccrual| -> Vec<(Option<i64>, Decimal)> {
            accrual
                .lines
                .iter()
                .filter(|line| line.kind == LineKind::Overtime)
                .map(|line| (line.seconds, line.multiplier.normalize()))
                .collect()
        };
        assert_eq!(overtime(&accruals[0]), [(hours(2), decimal("1.5"))], "an ordinary Monday: two hours over");
        assert_eq!(
            overtime(&accruals[1]),
            [(hours(2), decimal("1.5")), (hours(1), decimal("2"))],
            "the short eve: three hours over, the third at the second step"
        );

        // Half time: the norm is four hours, and so is the threshold.
        let half = Norm {
            standard_hours: Decimal::from(8),
            work_rate: decimal("0.5"),
        };
        let accruals = run(&Calendar::empty(), half, &versions, &[work("2026-11-09", hours(5))]);
        assert_eq!(overtime(&accruals[0]), [(hours(1), decimal("1.5"))]);
    }

    #[test]
    fn a_long_week_pays_its_weekly_step_on_the_hours_past_it() {
        // Five nine-hour days against forty a week: the last five hours of
        // Friday are over.
        let rules = Overtime {
            weekly: Some(after("40", "1.5")),
            ..Overtime::default()
        };
        let versions = [version(1, "2026-01-01", Basis::Hourly, "10", "USD", rules)];
        let days: Vec<Worked> = ["2026-11-09", "2026-11-10", "2026-11-11", "2026-11-12", "2026-11-13"]
            .iter()
            .map(|day| work(day, hours(9)))
            .collect();
        let accruals = run(&Calendar::empty(), full_time(), &versions, &days);
        for accrual in &accruals[..4] {
            assert_eq!(lines(accrual), [(LineKind::Hours, None, hours(9), Decimal::ONE, decimal("90"))]);
        }
        assert_eq!(
            lines(&accruals[4]),
            [
                (LineKind::Hours, None, hours(4), Decimal::ONE, decimal("40")),
                (LineKind::Overtime, Some(Rule::Weekly), hours(5), decimal("1.5"), decimal("75")),
            ]
        );
    }

    #[test]
    fn an_hour_is_never_paid_twice_for_being_over() {
        // A daily step at 8 and a weekly one at 40, five ten-hour days. The
        // two hours a day past eight are daily overtime, and only the eight
        // ordinary hours of each day count towards the week - which reaches
        // forty on Friday's last ordinary hour and never passes it.
        let rules = Overtime {
            daily: Some(after("8", "1.5")),
            weekly: Some(after("40", "1.5")),
            ..Overtime::default()
        };
        let versions = [version(1, "2026-01-01", Basis::Hourly, "10", "USD", rules)];
        let days: Vec<Worked> = ["2026-11-09", "2026-11-10", "2026-11-11", "2026-11-12", "2026-11-13"]
            .iter()
            .map(|day| work(day, hours(10)))
            .collect();
        let accruals = run(&Calendar::empty(), full_time(), &versions, &days);
        let overtime: i64 = accruals
            .iter()
            .flat_map(|accrual| &accrual.lines)
            .filter(|line| line.kind == LineKind::Overtime)
            .filter_map(|line| line.seconds)
            .sum();
        assert_eq!(overtime, 10 * 3600, "ten hours over, not ten plus the week's");
        assert!(
            accruals.iter().flat_map(|accrual| &accrual.lines).all(|line| line.rule != Some(Rule::Weekly)),
            "no hour is weekly overtime"
        );

        // A Saturday after that week is past forty from its first hour.
        let mut days = days;
        days.push(work("2026-11-14", hours(3)));
        let accruals = run(&Calendar::empty(), full_time(), &versions, &days);
        assert_eq!(
            lines(&accruals[5]),
            [(LineKind::Overtime, Some(Rule::Weekly), hours(3), decimal("1.5"), decimal("45"))]
        );
    }

    #[test]
    fn a_rest_day_takes_the_larger_multiplier_not_the_product() {
        // Rest days at 2, and a daily step at 1.5 past the norm - which on a
        // Saturday is nothing, so every hour is past it. Every hour pays 2.
        let rules = Overtime {
            daily: Some(past_norm("1.5")),
            rest_days: Some(decimal("2")),
            ..Overtime::default()
        };
        let versions = [version(1, "2026-01-01", Basis::Hourly, "10", "USD", rules)];
        let accruals = run(&Calendar::empty(), full_time(), &versions, &[work("2026-11-14", hours(5))]);
        assert!(accruals[0].rest_day);
        assert_eq!(
            lines(&accruals[0]),
            [(LineKind::Overtime, Some(Rule::RestDay), hours(5), decimal("2"), decimal("100"))]
        );

        // A step past the rest day's multiplier still wins where it reaches.
        let rules = Overtime {
            daily: Some(then(after("8", "1.5"), "2", "3")),
            rest_days: Some(decimal("2")),
            ..Overtime::default()
        };
        let versions = [version(1, "2026-01-01", Basis::Hourly, "10", "USD", rules)];
        let accruals = run(&Calendar::empty(), full_time(), &versions, &[work("2026-11-14", hours(11))]);
        assert_eq!(
            lines(&accruals[0]),
            [
                (LineKind::Overtime, Some(Rule::RestDay), hours(10), decimal("2"), decimal("200")),
                (LineKind::Overtime, Some(Rule::Daily), hours(1), decimal("3"), decimal("30")),
            ]
        );
    }

    #[test]
    fn a_holiday_is_a_rest_day_and_a_transferred_saturday_is_not() {
        let calendar = Calendar::from_days(vec![
            CalendarDay {
                date: date("2026-11-11"),
                kind: CalendarDayKind::Holiday,
                note: None,
            },
            CalendarDay {
                date: date("2026-11-14"),
                kind: CalendarDayKind::WorkingWeekend,
                note: None,
            },
        ]);
        let rules = Overtime {
            rest_days: Some(decimal("2")),
            ..Overtime::default()
        };
        let versions = [version(1, "2026-01-01", Basis::Hourly, "10", "USD", rules)];
        let accruals = run(&calendar, full_time(), &versions, &[work("2026-11-11", hours(4)), work("2026-11-14", hours(4))]);
        assert!(accruals[0].rest_day);
        assert_eq!(accruals[0].amount, Some(decimal("80")));
        assert!(!accruals[1].rest_day, "the calendar moved Saturday into the week");
        assert_eq!(accruals[1].amount, Some(decimal("40")));
    }

    #[test]
    fn a_day_rate_pays_the_share_worked_and_no_more() {
        let versions = [version(1, "2026-01-01", Basis::Daily, "200", "USD", Overtime::default())];
        let accruals = run(
            &Calendar::empty(),
            full_time(),
            &versions,
            &[work("2026-11-09", Some(20 * 60)), work("2026-11-10", hours(6)), work("2026-11-11", hours(11))],
        );
        // Twenty minutes is not a day.
        assert_eq!(accruals[0].lines[0].share, Some(decimal("0.0417")));
        assert_eq!(accruals[0].amount, Some(decimal("8.34")), "the share on the line times the rate");
        assert_eq!(accruals[1].amount, Some(decimal("150")));
        assert_eq!(accruals[2].amount, Some(decimal("200")), "eleven hours are one day without an overtime rule");
    }

    #[test]
    fn a_day_rate_pays_overtime_at_its_hourly_equivalent() {
        // 200 a day of eight hours is 25 an hour. Eleven hours with 1.5 past
        // the norm: the day, and three hours at 37.50.
        let rules = Overtime {
            daily: Some(past_norm("1.5")),
            ..Overtime::default()
        };
        let versions = [version(1, "2026-01-01", Basis::Daily, "200", "USD", rules)];
        let accruals = run(&Calendar::empty(), full_time(), &versions, &[work("2026-11-09", hours(11))]);
        assert_eq!(
            lines(&accruals[0]),
            [
                (LineKind::Day, None, None, Decimal::ONE, decimal("200")),
                (LineKind::Overtime, Some(Rule::Daily), hours(3), decimal("1.5"), decimal("112.50")),
            ]
        );
        assert_eq!(accruals[0].lines[1].rate, decimal("25"));

        // A threshold inside the day: hours six to eight are paid by the day
        // and earn only the half their multiplier adds; eight to nine earn
        // all of it.
        let rules = Overtime {
            daily: Some(after("6", "1.5")),
            ..Overtime::default()
        };
        let versions = [version(1, "2026-01-01", Basis::Daily, "200", "USD", rules)];
        let accruals = run(&Calendar::empty(), full_time(), &versions, &[work("2026-11-09", hours(9))]);
        assert_eq!(
            lines(&accruals[0]),
            [
                (LineKind::Day, None, None, Decimal::ONE, decimal("200")),
                (LineKind::OvertimePremium, Some(Rule::Daily), hours(2), decimal("0.5"), decimal("25")),
                (LineKind::Overtime, Some(Rule::Daily), hours(1), decimal("1.5"), decimal("37.50")),
            ]
        );
    }

    #[test]
    fn a_day_rate_on_a_rest_day_is_measured_against_the_persons_own_day() {
        // A Saturday owes nothing, and a share of nothing is undefined: four
        // hours of a half-timer's four-hour day is the whole day.
        let half = Norm {
            standard_hours: Decimal::from(8),
            work_rate: decimal("0.5"),
        };
        let versions = [version(1, "2026-01-01", Basis::Daily, "100", "USD", Overtime::default())];
        let accruals = run(&Calendar::empty(), half, &versions, &[work("2026-11-14", hours(2))]);
        assert_eq!(accruals[0].amount, Some(decimal("50")));

        // Nobody's day is zero: with no share at all, the installation's.
        let none = Norm {
            standard_hours: Decimal::from(8),
            work_rate: Decimal::ZERO,
        };
        let accruals = run(&Calendar::empty(), none, &versions, &[work("2026-11-09", hours(2))]);
        assert_eq!(accruals[0].amount, Some(decimal("25")));
    }

    #[test]
    fn what_is_not_accrued_is_said_not_drawn_as_zero() {
        let versions = [
            version(1, "2026-11-10", Basis::Hourly, "10", "USD", Overtime::default()),
            version(2, "2026-11-12", Basis::Monthly, "5000", "USD", Overtime::default()),
        ];
        let away = Worked {
            date: date("2026-11-11"),
            kind: WorkdayKind::Vacation,
            worked_seconds: Some(0),
        };
        let accruals = run(
            &Calendar::empty(),
            full_time(),
            &versions,
            &[work("2026-11-09", hours(8)), work("2026-11-10", None), away, work("2026-11-12", hours(8))],
        );
        let statuses: Vec<Status> = accruals.iter().map(|accrual| accrual.status).collect();
        assert_eq!(statuses, [Status::NoTerms, Status::Open, Status::Away, Status::Salaried]);
        assert_eq!(amounts(&accruals), [None, None, None, None]);
    }

    #[test]
    fn a_raise_mid_week_prices_each_date_by_its_own_version() {
        let rules = Overtime {
            weekly: Some(after("40", "1.5")),
            ..Overtime::default()
        };
        let versions = [
            version(1, "2026-01-01", Basis::Hourly, "10", "USD", rules),
            version(2, "2026-11-12", Basis::Hourly, "12", "USD", rules),
        ];
        let days: Vec<Worked> = ["2026-11-09", "2026-11-10", "2026-11-11", "2026-11-12", "2026-11-13"]
            .iter()
            .map(|day| work(day, hours(9)))
            .collect();
        let accruals = run(&Calendar::empty(), full_time(), &versions, &days);
        assert_eq!(accruals[2].amount, Some(decimal("90")), "Wednesday at the old rate");
        assert_eq!(accruals[3].amount, Some(decimal("108")), "Thursday at the new");
        // Friday: four ordinary hours at 12, five over at 18. The days before
        // the raise count towards the week like any others.
        assert_eq!(accruals[4].amount, Some(decimal("138")));
    }

    #[test]
    fn a_date_earns_the_same_whichever_range_asks() {
        // The property the handler's whole-week read exists for. Accrue
        // twelve weeks at once, then every week and every single date alone
        // (each with its own week read in full, as the handler does): every
        // date must come to the same amount, and the parts to the whole.
        let rules = Overtime {
            daily: Some(then(past_norm("1.5"), "2", "2")),
            weekly: Some(after("40", "1.25")),
            rest_days: Some(decimal("2")),
        };
        let versions = [
            version(1, "2026-01-01", Basis::Hourly, "13.37", "USD", rules),
            version(2, "2026-10-01", Basis::Daily, "211.11", "USD", rules),
        ];
        let start = date("2026-09-07");
        let days: Vec<Worked> = (0..84)
            .map(|offset| {
                let day = start + Duration::days(offset);
                // An irregular week: some long days, some short, a Saturday.
                let worked = (offset * 7919) % (11 * 3600) + if offset % 7 == 5 { 0 } else { 3 * 3600 };
                Worked {
                    date: day,
                    kind: WorkdayKind::Work,
                    worked_seconds: Some(worked),
                }
            })
            .filter(|day| day.date.weekday() != chrono::Weekday::Sun)
            .collect();
        let calendar = Calendar::empty();
        let whole = run(&calendar, full_time(), &versions, &days);

        for day in &days {
            let week: Vec<Worked> = days.iter().filter(|other| monday_of(other.date) == monday_of(day.date)).copied().collect();
            let alone = run(&calendar, full_time(), &versions, &week);
            let mine = alone.iter().find(|accrual| accrual.date == day.date).unwrap();
            let in_whole = whole.iter().find(|accrual| accrual.date == day.date).unwrap();
            assert_eq!(mine.amount, in_whole.amount, "{} earns the same alone", day.date);
            assert_eq!(mine.lines, in_whole.lines);
        }

        let total: Decimal = whole.iter().filter_map(|accrual| accrual.amount).sum();
        let by_month: Decimal = [9, 10, 11]
            .iter()
            .map(|month| {
                whole
                    .iter()
                    .filter(|accrual| accrual.date.month() == *month)
                    .filter_map(|accrual| accrual.amount)
                    .sum::<Decimal>()
            })
            .sum();
        assert_eq!(by_month, total, "the months add up to the whole, to the cent");
    }

    #[test]
    fn every_line_is_its_units_times_its_rate_times_its_multiplier() {
        // What lets a person check a line by hand.
        let rules = Overtime {
            daily: Some(then(after("6", "1.5"), "3", "2")),
            ..Overtime::default()
        };
        for basis in [Basis::Hourly, Basis::Daily] {
            let versions = [version(1, "2026-01-01", basis, "123.4567", "USD", rules)];
            let accruals = run(&Calendar::empty(), full_time(), &versions, &[work("2026-11-09", Some(10 * 3600 + 1234))]);
            assert!(accruals[0].lines.len() > 2, "{basis:?} has lines of every kind to check");
            for line in &accruals[0].lines {
                let product = match (line.seconds, line.share) {
                    (Some(seconds), None) => Decimal::from(seconds) * line.rate * line.multiplier / Decimal::from(3600),
                    (None, Some(share)) => share * line.rate * line.multiplier,
                    other => panic!("a line is hours or a share, not {other:?}"),
                };
                assert_eq!(line.amount, round(product, 2), "{line:?}");
            }
        }
    }

    #[test]
    fn amounts_round_to_the_currency_and_half_a_cent_goes_up() {
        assert_eq!(round(decimal("10.005"), 2), decimal("10.01"));
        assert_eq!(round(decimal("10.004"), 2), decimal("10.00"));
        assert_eq!(minor_unit("JPY"), 0);
        assert_eq!(minor_unit("BHD"), 3);
        let versions = [version(1, "2026-01-01", Basis::Hourly, "1000", "JPY", Overtime::default())];
        let accruals = run(&Calendar::empty(), full_time(), &versions, &[work("2026-11-09", Some(1801))]);
        assert_eq!(accruals[0].amount, Some(decimal("500")), "yen have no minor unit");
    }

    #[test]
    fn totals_keep_each_currency_apart() {
        let versions = [
            version(1, "2026-01-01", Basis::Hourly, "10", "USD", Overtime::default()),
            version(2, "2026-11-11", Basis::Hourly, "9", "EUR", Overtime::default()),
        ];
        let accruals = run(
            &Calendar::empty(),
            full_time(),
            &versions,
            &[work("2026-11-09", hours(8)), work("2026-11-10", hours(8)), work("2026-11-11", hours(8))],
        );
        let range = Range {
            from: date("2026-11-09"),
            to: date("2026-11-15"),
        };
        let summary = summarize(Uuid::nil(), &range, accruals);
        let totals: Vec<(&str, &str)> = summary.totals.iter().map(|total| (total.currency.as_str(), total.amount.as_str())).collect();
        assert_eq!(totals, [("EUR", "72.00"), ("USD", "160.00")]);
    }

    #[test]
    fn the_wire_names_are_the_contract() {
        assert_eq!(serde_json::to_value(Status::NoTerms).unwrap(), "no_terms");
        assert_eq!(serde_json::to_value(Status::Salaried).unwrap(), "salaried");
        assert_eq!(serde_json::to_value(Rule::RestDay).unwrap(), "rest_day");
        assert_eq!(serde_json::to_value(LineKind::OvertimePremium).unwrap(), "overtime_premium");
    }
}
