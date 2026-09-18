//! The production calendar and the norm: how much a date was meant to be
//! worked, and by whom.
//!
//! This is the first standard the server holds. Everything before it compared
//! a person with their own history and said so (ADR 0016), because there was
//! nothing else to compare them with; the heatmap left the notion of a full
//! day to the client for the same reason (ADR 0015).
//!
//! Three facts make the norm, and each is stored exactly once (ADR 0017):
//!
//! * **The calendar** holds only the dates that differ from the weekday they
//!   fall on - a holiday, a shortened eve, a Saturday moved into the working
//!   week. A list of holidays could only subtract, and a production calendar
//!   also adds.
//! * **The installation's full day** (`settings.standard_hours`) says what a
//!   whole day of work is here.
//! * **A person's share of it** (`users.work_rate`) says whose day is half
//!   that, and stays half when the other two change.
//!
//! The norm itself is derived from them on every read. Storing it would put a
//! second copy of a computed fact next to the rows that produce it, and the
//! copy would survive a correction to the calendar that the derivation simply
//! absorbs.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::{Datelike, NaiveDate, Weekday};
use rust_decimal::{Decimal, prelude::ToPrimitive};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{app::AppState, audit, error::ApiError, login::CurrentUser, me::Range};

/// Seconds in an hour, as a decimal, so an hour figure becomes seconds without
/// going through a float on the way.
const SECONDS_PER_HOUR: i64 = 3600;

/// How much a short day is shortened by, in hours.
///
/// One, everywhere this rule exists. Not a setting: an installation that needs
/// a different figure needs a different rule, and a knob here would ask the
/// operator to invent a calendar convention rather than record one.
const SHORT_DAY_RELIEF_HOURS: i64 = 1;

/// What makes a date unlike the weekday it falls on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "calendar_day_kind", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum CalendarDayKind {
    /// A working weekday that is not worked.
    Holiday,
    /// The eve of a holiday: one hour shorter.
    ShortDay,
    /// A weekend day moved into the working week, usually because a holiday
    /// was transferred off it.
    WorkingWeekend,
}

/// What kind of day an employee had, as their agent reported it.
///
/// Optional on the wire and defaulting to `Work`: an agent that predates the
/// field says `work` by saying nothing (ADR 0004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "workday_kind", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum WorkdayKind {
    #[default]
    Work,
    Vacation,
    Sick,
    DayOff,
}

impl WorkdayKind {
    /// Whether a day of this kind owes the norm.
    ///
    /// Leave and illness owe nothing. Without this a fortnight of holiday
    /// reads as eighty hours missing, which is the most alarming possible way
    /// to be wrong about somebody on a beach.
    pub fn owes_the_norm(self) -> bool {
        matches!(self, Self::Work)
    }
}

/// One dated exception, as stored and as the API answers it.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CalendarDay {
    pub date: NaiveDate,
    pub kind: CalendarDayKind,
    /// What the day is called, for the screen that lists a year. The date and
    /// the kind are the calendar; the name is for people.
    pub note: Option<String>,
}

/// The calendar over a span of dates, as a lookup.
///
/// Loaded once per request rather than queried per day: a month view asks
/// about thirty dates, and thirty round trips for a dozen rows is the shape of
/// query that looks fine on a laptop and shows up on a dashboard.
#[derive(Debug, Clone, Default)]
pub struct Calendar {
    /// Sorted by date, so a lookup binary-searches rather than hashing.
    days: Vec<CalendarDay>,
}

impl Calendar {
    /// Loads the exceptions between two dates, both ends inclusive.
    pub async fn load(pool: &PgPool, from: NaiveDate, to: NaiveDate) -> Result<Self, ApiError> {
        let days: Vec<CalendarDay> = sqlx::query_as("SELECT date, kind, note FROM calendar_days WHERE date BETWEEN $1 AND $2 ORDER BY date")
            .bind(from)
            .bind(to)
            .fetch_all(pool)
            .await?;
        Ok(Self { days })
    }

    /// The calendar with nothing in it: every weekday a full day, every
    /// weekend off. What an installation that has entered no calendar gets.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Builds one from rows already in hand - the demo seed, and the tests.
    pub fn from_days(mut days: Vec<CalendarDay>) -> Self {
        days.sort_by_key(|day| day.date);
        Self { days }
    }

    /// The exceptions themselves, for a caller that lists them.
    pub fn days(&self) -> &[CalendarDay] {
        &self.days
    }

    fn kind_of(&self, date: NaiveDate) -> Option<CalendarDayKind> {
        self.days.binary_search_by_key(&date, |day| day.date).ok().map(|at| self.days[at].kind)
    }

    /// The hours a date is worth at full rate.
    ///
    /// The whole rule in one place (ADR 0017): a weekend is nothing unless the
    /// calendar moved it into the week, a holiday is nothing, an eve is an
    /// hour less, and everything else is a full day.
    pub fn hours_on(&self, date: NaiveDate, standard_hours: Decimal) -> Decimal {
        match self.kind_of(date) {
            Some(CalendarDayKind::Holiday) => Decimal::ZERO,
            Some(CalendarDayKind::ShortDay) => (standard_hours - Decimal::from(SHORT_DAY_RELIEF_HOURS)).max(Decimal::ZERO),
            Some(CalendarDayKind::WorkingWeekend) => standard_hours,
            None if is_weekend(date) => Decimal::ZERO,
            None => standard_hours,
        }
    }

    /// The seconds one person owes on a date.
    ///
    /// The rate multiplies last, and the rounding happens once at the end: a
    /// half-rate short day is `(8 - 1) x 0.5`, not a rounded seven halved.
    pub fn norm_seconds(&self, date: NaiveDate, standard_hours: Decimal, work_rate: Decimal) -> i64 {
        to_seconds(self.hours_on(date, standard_hours) * work_rate)
    }

    /// The seconds one person owes across a range, both ends inclusive.
    ///
    /// Days the employee was away are the caller's business: this counts what
    /// the calendar asks for, and [`Norm::for_range`] subtracts the leave.
    pub fn norm_seconds_over(&self, from: NaiveDate, to: NaiveDate, standard_hours: Decimal, work_rate: Decimal) -> i64 {
        let mut total = 0;
        let mut date = from;
        while date <= to {
            total += self.norm_seconds(date, standard_hours, work_rate);
            let Some(next) = date.succ_opt() else { break };
            date = next;
        }
        total
    }
}

/// Whether a date falls on a weekend, before the calendar has its say.
fn is_weekend(date: NaiveDate) -> bool {
    matches!(date.weekday(), Weekday::Sat | Weekday::Sun)
}

/// Hours to whole seconds.
///
/// Rounded rather than truncated: a rate of a third of a day would otherwise
/// lose a second per day and a minute per year, always in the same direction.
fn to_seconds(hours: Decimal) -> i64 {
    (hours * Decimal::from(SECONDS_PER_HOUR)).round().to_i64().unwrap_or(0)
}

/// The installation's full day and one person's share of it.
///
/// Carried together because neither means anything alone: the hours without
/// the rate report a half-time employee as half a person, and the rate without
/// the hours is a fraction of nothing.
#[derive(Debug, Clone, Copy)]
pub struct Norm {
    pub standard_hours: Decimal,
    pub work_rate: Decimal,
}

impl Norm {
    /// Reads the installation's full day and one person's rate.
    pub async fn load(pool: &PgPool, user_id: Uuid) -> Result<Self, ApiError> {
        let standard_hours = Self::standard_hours(pool).await?;
        let work_rate: Decimal = sqlx::query_scalar("SELECT work_rate FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(pool)
            .await?
            .unwrap_or(Decimal::ONE);
        Ok(Self { standard_hours, work_rate })
    }

    /// The installation's full day alone, for a caller that holds the rates
    /// itself - the team table reads one rate per row.
    pub async fn standard_hours(pool: &PgPool) -> Result<Decimal, ApiError> {
        Ok(sqlx::query_scalar("SELECT standard_hours FROM settings WHERE singleton")
            .fetch_one(pool)
            .await?)
    }

    /// What one person owes over a range, with the days they were away taken
    /// out of it.
    ///
    /// `away` are the dates whose stored day is leave or illness. They are
    /// removed from the norm rather than counted as worked: a week of holiday
    /// is a week that owes nothing, not a week worked in full.
    pub fn for_range(&self, calendar: &Calendar, from: NaiveDate, to: NaiveDate, away: &[NaiveDate]) -> i64 {
        let full = calendar.norm_seconds_over(from, to, self.standard_hours, self.work_rate);
        let excused: i64 = away
            .iter()
            .filter(|date| **date >= from && **date <= to)
            .map(|date| calendar.norm_seconds(*date, self.standard_hours, self.work_rate))
            .sum();
        (full - excused).max(0)
    }
}

/// What the norm endpoints answer alongside hours worked.
///
/// A pair, never a percentage: the screen divides. A server that answered
/// "80%" would have decided that eight hours out of ten is the same fact as
/// four out of five, and thrown away the two numbers a person reads.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Progress {
    /// Seconds the calendar and the person's rate ask for, over the range,
    /// less the days they were on leave.
    pub norm_seconds: i64,
    /// The installation's full day, in hours, so a screen can say what a day
    /// is worth without a second request.
    pub standard_hours: Decimal,
    /// This person's share of it.
    pub work_rate: Decimal,
}

// The administrative API ------------------------------------------------------

/// The year being asked for. A year at a time is how a calendar is published
/// and how an administrator checks one.
#[derive(Debug, Deserialize)]
pub struct YearQuery {
    pub year: i32,
}

/// The calendar of a year, with the installation's full day beside it.
#[derive(Debug, Serialize)]
pub struct CalendarYear {
    pub year: i32,
    pub days: Vec<CalendarDay>,
    /// The full day this installation works, so a screen showing the calendar
    /// does not need a second request to say what a day is worth.
    pub standard_hours: Decimal,
}

/// A day being entered or corrected.
#[derive(Debug, Deserialize)]
pub struct CalendarDayInput {
    pub date: NaiveDate,
    pub kind: CalendarDayKind,
    #[serde(default)]
    pub note: Option<String>,
}

/// A year's worth of exceptions, replacing whatever that year held.
///
/// Whole years rather than day by day, because that is how the source document
/// arrives: a decree publishes a year, and an administrator who has to add
/// eleven days one at a time will get one of them wrong.
#[derive(Debug, Deserialize)]
pub struct CalendarYearInput {
    pub days: Vec<CalendarDayInput>,
}

/// The installation's full day being set.
#[derive(Debug, Deserialize)]
pub struct StandardHoursInput {
    pub standard_hours: Decimal,
}

/// A person's share of a full day being set.
#[derive(Debug, Deserialize)]
pub struct WorkRateInput {
    pub work_rate: Decimal,
}

/// Answers a year of the calendar.
///
/// Readable by anyone signed in: which days of the year are worked is not a
/// secret from the people working them, and the employee's own screen shows
/// their norm beside their hours.
pub async fn year(State(state): State<AppState>, _user: CurrentUser, Query(query): Query<YearQuery>) -> Result<impl IntoResponse, ApiError> {
    let (from, to) = year_bounds(query.year)?;
    let calendar = Calendar::load(&state.pool, from, to).await?;

    Ok(Json(CalendarYear {
        year: query.year,
        days: calendar.days,
        standard_hours: Norm::standard_hours(&state.pool).await?,
    }))
}

/// Replaces a year of the calendar. Administrators only, and recorded.
///
/// A replacement rather than a merge: a corrected calendar is the document
/// that is right, and merging would leave last week's wrong rows in place with
/// nothing on the screen to say they are still there.
pub async fn put_year(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<YearQuery>,
    Json(input): Json<CalendarYearInput>,
) -> Result<impl IntoResponse, ApiError> {
    user.require_admin()?;
    let (from, to) = year_bounds(query.year)?;

    for (index, day) in input.days.iter().enumerate() {
        if day.date < from || day.date > to {
            return Err(ApiError::bad_request(format!("days[{index}]: {} is not in {}", day.date, query.year)));
        }
    }

    // All of it or none: a half-written year is a calendar nobody can check
    // against the document it came from.
    let mut tx = state.pool.begin().await?;

    sqlx::query("DELETE FROM calendar_days WHERE date BETWEEN $1 AND $2")
        .bind(from)
        .bind(to)
        .execute(&mut *tx)
        .await?;

    for day in &input.days {
        sqlx::query("INSERT INTO calendar_days (date, kind, note) VALUES ($1, $2, $3)")
            .bind(day.date)
            .bind(day.kind)
            .bind(day.note.as_deref().map(str::trim).filter(|note| !note.is_empty()))
            .execute(&mut *tx)
            .await
            // A duplicate date is the administrator's typo, not a server
            // fault: the primary key catches it, and a 500 would tell them
            // nothing about which day they entered twice.
            .map_err(|error| match &error {
                sqlx::Error::Database(db) if db.is_unique_violation() => ApiError::bad_request(format!("{} appears twice", day.date)),
                _ => ApiError::from(error),
            })?;
    }

    tx.commit().await?;

    tracing::info!(year = query.year, days = input.days.len(), by = %user.user_id, "replaced a year of the calendar");
    audit::Entry::new(audit::action::CALENDAR_YEAR_REPLACED)
        .by(user.user_id)
        .by_email(&user.email)
        .with(serde_json::json!({ "year": query.year, "days": input.days.len() }))
        .record(&state.pool)
        .await;

    let calendar = Calendar::load(&state.pool, from, to).await?;
    Ok((
        StatusCode::OK,
        Json(CalendarYear {
            year: query.year,
            days: calendar.days,
            standard_hours: Norm::standard_hours(&state.pool).await?,
        }),
    ))
}

/// Sets the installation's full day. Administrators only, and recorded.
pub async fn put_standard_hours(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(input): Json<StandardHoursInput>,
) -> Result<impl IntoResponse, ApiError> {
    user.require_admin()?;

    if input.standard_hours <= Decimal::ZERO || input.standard_hours > Decimal::from(24) {
        return Err(ApiError::bad_request("a full day is more than zero hours and at most 24"));
    }

    let previous: Decimal = sqlx::query_scalar("SELECT standard_hours FROM settings WHERE singleton")
        .fetch_one(&state.pool)
        .await?;

    sqlx::query("UPDATE settings SET standard_hours = $1 WHERE singleton")
        .bind(input.standard_hours)
        .execute(&state.pool)
        .await?;

    tracing::info!(from = %previous, to = %input.standard_hours, by = %user.user_id, "changed the installation's full day");
    // The figure every norm on every screen is computed from: a change to it
    // moves everybody's numbers at once, and has to leave a trace saying who.
    audit::Entry::new(audit::action::STANDARD_HOURS_CHANGED)
        .by(user.user_id)
        .by_email(&user.email)
        .with(serde_json::json!({ "from": previous, "to": input.standard_hours }))
        .record(&state.pool)
        .await;

    Ok((StatusCode::OK, Json(serde_json::json!({ "standard_hours": input.standard_hours }))))
}

/// Sets one person's share of a full day. Administrators only, and recorded.
///
/// Its own route rather than a field on the user patch: this is the one
/// attribute of an account that changes what every screen says about the
/// person, and an audit entry naming it is easier to find than one that says
/// "user updated".
pub async fn put_work_rate(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(target): Path<Uuid>,
    Json(input): Json<WorkRateInput>,
) -> Result<impl IntoResponse, ApiError> {
    user.require_admin()?;

    if input.work_rate < Decimal::ZERO || input.work_rate > Decimal::from(2) {
        return Err(ApiError::bad_request("a share of a full day is between 0 and 2"));
    }

    // The previous value comes back from the same statement that replaces it:
    // reading it first would let two administrators record each other's
    // starting point.
    let previous: Option<Decimal> = sqlx::query_scalar(
        "UPDATE users SET work_rate = $1 FROM (SELECT work_rate FROM users WHERE id = $2) AS before
         WHERE users.id = $2 RETURNING before.work_rate",
    )
    .bind(input.work_rate)
    .bind(target)
    .fetch_optional(&state.pool)
    .await?;

    let Some(previous) = previous else {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "no such user"));
    };

    tracing::info!(user = %target, from = %previous, to = %input.work_rate, by = %user.user_id, "changed a work rate");
    audit::Entry::new(audit::action::WORK_RATE_CHANGED)
        .by(user.user_id)
        .by_email(&user.email)
        .on(target)
        .with(serde_json::json!({ "from": previous, "to": input.work_rate }))
        .record(&state.pool)
        .await;

    Ok((StatusCode::OK, Json(serde_json::json!({ "work_rate": input.work_rate }))))
}

/// The first and last date of a year, refusing one the calendar cannot hold.
fn year_bounds(year: i32) -> Result<(NaiveDate, NaiveDate), ApiError> {
    let from = NaiveDate::from_ymd_opt(year, 1, 1).ok_or_else(|| ApiError::bad_request(format!("{year} is not a year")))?;
    let to = NaiveDate::from_ymd_opt(year, 12, 31).ok_or_else(|| ApiError::bad_request(format!("{year} is not a year")))?;
    Ok((from, to))
}

/// The dates in a range a person was away, so a norm can excuse them.
pub async fn away_dates(pool: &PgPool, user_id: Uuid, range: &Range) -> Result<Vec<NaiveDate>, ApiError> {
    let dates: Vec<NaiveDate> = sqlx::query_scalar("SELECT date FROM workdays WHERE user_id = $1 AND date BETWEEN $2 AND $3 AND kind <> 'work' ORDER BY date")
        .bind(user_id)
        .bind(range.from)
        .bind(range.to)
        .fetch_all(pool)
        .await?;
    Ok(dates)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> NaiveDate {
        text.parse().expect("a test date")
    }

    fn day(text: &str, kind: CalendarDayKind) -> CalendarDay {
        CalendarDay {
            date: date(text),
            kind,
            note: None,
        }
    }

    fn eight() -> Decimal {
        Decimal::from(8)
    }

    #[test]
    fn an_empty_calendar_works_the_weekdays() {
        let calendar = Calendar::empty();
        // 2026-09-14 is a Monday, 2026-09-19 a Saturday.
        assert_eq!(calendar.hours_on(date("2026-09-14"), eight()), eight());
        assert_eq!(calendar.hours_on(date("2026-09-19"), eight()), Decimal::ZERO);
        assert_eq!(calendar.hours_on(date("2026-09-20"), eight()), Decimal::ZERO);
    }

    #[test]
    fn a_holiday_is_worth_nothing_and_an_eve_an_hour_less() {
        let calendar = Calendar::from_days(vec![day("2026-01-01", CalendarDayKind::Holiday), day("2025-12-31", CalendarDayKind::ShortDay)]);
        assert_eq!(calendar.hours_on(date("2026-01-01"), eight()), Decimal::ZERO);
        assert_eq!(calendar.hours_on(date("2025-12-31"), eight()), Decimal::from(7));
    }

    #[test]
    fn a_working_weekend_is_a_full_day() {
        // The case a list of holidays cannot express: a calendar also adds
        // days, and without this the transferred Saturday owes nothing while
        // the whole team is at work (ADR 0017).
        let saturday = date("2026-11-07");
        assert_eq!(saturday.weekday(), Weekday::Sat);

        assert_eq!(Calendar::empty().hours_on(saturday, eight()), Decimal::ZERO);
        let calendar = Calendar::from_days(vec![day("2026-11-07", CalendarDayKind::WorkingWeekend)]);
        assert_eq!(calendar.hours_on(saturday, eight()), eight());
    }

    #[test]
    fn the_rate_multiplies_the_shortened_day_not_the_other_way() {
        // Half of a seven-hour eve is three and a half hours. Applying the
        // relief after the rate would give three, and the difference is half
        // an hour of somebody's month every time a short day comes round.
        let calendar = Calendar::from_days(vec![day("2026-12-31", CalendarDayKind::ShortDay)]);
        let half = Decimal::new(5, 1);
        assert_eq!(calendar.norm_seconds(date("2026-12-31"), eight(), half), 3 * 3600 + 1800);
    }

    #[test]
    fn a_week_sums_its_days() {
        // Monday to Sunday: five full days, the weekend nothing.
        let total = Calendar::empty().norm_seconds_over(date("2026-09-14"), date("2026-09-20"), eight(), Decimal::ONE);
        assert_eq!(total, 5 * 8 * 3600);
    }

    #[test]
    fn a_holiday_takes_its_day_out_of_the_week() {
        let calendar = Calendar::from_days(vec![day("2026-09-16", CalendarDayKind::Holiday)]);
        let total = calendar.norm_seconds_over(date("2026-09-14"), date("2026-09-20"), eight(), Decimal::ONE);
        assert_eq!(total, 4 * 8 * 3600, "the week owes four days once a Wednesday is a holiday");
    }

    #[test]
    fn leave_is_excused_rather_than_counted_as_worked() {
        // The defect this guards: a fortnight of holiday reported as eighty
        // hours missing.
        let norm = Norm {
            standard_hours: eight(),
            work_rate: Decimal::ONE,
        };
        let calendar = Calendar::empty();
        let (monday, sunday) = (date("2026-09-14"), date("2026-09-20"));

        assert_eq!(norm.for_range(&calendar, monday, sunday, &[]), 5 * 8 * 3600);
        assert_eq!(
            norm.for_range(&calendar, monday, sunday, &[date("2026-09-15"), date("2026-09-16")]),
            3 * 8 * 3600,
            "two days of leave are not owed"
        );
    }

    #[test]
    fn a_weekend_of_leave_excuses_nothing_it_did_not_owe() {
        // Taking Saturday off cannot reduce a norm that never asked for it -
        // otherwise a day of leave on a weekend would quietly credit the week
        // with eight hours nobody was due to work.
        let norm = Norm {
            standard_hours: eight(),
            work_rate: Decimal::ONE,
        };
        let total = norm.for_range(&Calendar::empty(), date("2026-09-14"), date("2026-09-20"), &[date("2026-09-19")]);
        assert_eq!(total, 5 * 8 * 3600);
    }

    #[test]
    fn a_day_the_employee_was_away_owes_nothing() {
        assert!(WorkdayKind::Work.owes_the_norm());
        for away in [WorkdayKind::Vacation, WorkdayKind::Sick, WorkdayKind::DayOff] {
            assert!(!away.owes_the_norm(), "{away:?} owes no hours");
        }
    }

    #[test]
    fn the_wire_names_are_snake_case() {
        // Part of the contract with kasl and the web UI: a rename here is a
        // breaking change, so it fails a test rather than a client.
        assert_eq!(serde_json::to_string(&WorkdayKind::DayOff).unwrap(), "\"day_off\"");
        assert_eq!(serde_json::to_string(&CalendarDayKind::WorkingWeekend).unwrap(), "\"working_weekend\"");
        assert_eq!(serde_json::to_string(&CalendarDayKind::ShortDay).unwrap(), "\"short_day\"");
    }

    #[test]
    fn an_agent_that_says_nothing_worked() {
        // The compatibility hinge, as with `tasks_are_complete` before it: a
        // kasl too old to know the field must not have its silence read as
        // "this person was on holiday".
        assert_eq!(WorkdayKind::default(), WorkdayKind::Work);
        assert_eq!(serde_json::from_str::<WorkdayKind>("\"work\"").unwrap(), WorkdayKind::Work);
    }

    #[test]
    fn a_year_outside_the_calendar_is_refused() {
        assert!(year_bounds(2026).is_ok());
        let error = year_bounds(i32::MAX).unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }
}
