//! Pay terms: how a person is paid, version by version (ADR 0027).
//!
//! `GET` and `POST /api/v1/users/{id}/pay-terms` for an administrator, and
//! `GET /api/v1/me/pay-terms` for the person the terms are about.
//!
//! Terms are a history that only grows. A version names the first date it
//! applies to and no last one - it ends where the next begins, so two versions
//! can neither overlap nor leave a gap. A version is never rewritten: a raise
//! is a new version, and a mistake is corrected by a new version with the same
//! start, which takes the date over while the one it corrects stays readable
//! as `superseded`. The database refuses an update or a direct delete outright,
//! so the rule does not rest on every later handler remembering it.
//!
//! Which version applies to a date is [`in_force`], and nothing else may answer
//! that question: every accrual computed from here on asks it.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::{DateTime, NaiveDate, Utc};
use iso_currency::Currency;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{app::AppState, audit, calendar::Norm, error::ApiError, login::CurrentUser};

/// What a rate is a rate of. Mirrors the `pay_basis` enum.
///
/// Piecework and one-off bonuses are not here: they are layers added on top of
/// a basis, so pay that mixes a salary with piece rates is a `monthly` basis
/// with a layer, not a value that would have to say which mix (ADR 0027).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "pay_basis", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum Basis {
    /// Per hour worked.
    Hourly,
    /// Per day worked.
    Daily,
    /// Per month, prorated by the norm.
    Monthly,
}

/// The most decimal places a rate may carry, which is also the scale of the
/// column. A fifth is refused rather than rounded away by the column unseen.
pub const RATE_SCALE: u32 = 4;

/// Digits before the decimal point the column holds: `numeric(18, 4)`.
const RATE_INTEGER_DIGITS: u32 = 14;

/// The longest note a version may carry, as the column checks it.
const NOTE_MAX_CHARS: usize = 500;

/// One version of a person's terms, as stored.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Version {
    /// Also the order of recording: of two versions with one start, the one
    /// with the larger id is in force.
    pub id: i64,
    pub user_id: Uuid,
    pub effective_from: NaiveDate,
    pub basis: Basis,
    pub rate: Decimal,
    pub currency: String,
    pub note: Option<String>,
    pub recorded_by: Uuid,
    /// The recorder's display name, read with the version.
    pub recorded_by_name: String,
    pub recorded_at: DateTime<Utc>,
}

/// The version that applies to `date`, if any does.
///
/// Of the versions starting on or before the date, the one with the latest
/// start; of two with the same start, the one recorded later. Before the first
/// version there are no terms, and that is answered as `None` - never as a
/// zero rate, which would read as a person paid nothing rather than a person
/// whose terms were never entered.
pub fn in_force(versions: &[Version], date: NaiveDate) -> Option<&Version> {
    versions
        .iter()
        .filter(|version| version.effective_from <= date)
        .max_by_key(|version| (version.effective_from, version.id))
}

/// Where a version stands on a given day.
///
/// Worked out here rather than on a screen: which version is current is
/// [`in_force`], and a client deriving it again would be a second copy of the
/// one rule every accrual depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Standing {
    /// Applies today.
    Current,
    /// Starts after today.
    Upcoming,
    /// Applied, until a version with a later start took over.
    Ended,
    /// Never applies: a version recorded later with the same start corrected
    /// it. Kept, because a correction that erased what it corrected would also
    /// erase the reason a month once came to a different figure.
    Superseded,
}

/// The state of `version` among `versions` on `today`.
pub fn state_of(version: &Version, versions: &[Version], today: NaiveDate) -> Standing {
    let corrected = versions
        .iter()
        .any(|other| other.effective_from == version.effective_from && other.id > version.id);
    if corrected {
        return Standing::Superseded;
    }
    if version.effective_from > today {
        return Standing::Upcoming;
    }
    match in_force(versions, today) {
        Some(current) if current.id == version.id => Standing::Current,
        _ => Standing::Ended,
    }
}

/// A rate as the API writes it: exact, as a string, with the currency's minor
/// unit shown and nothing beyond what the rate carries - `25.50` dollars,
/// `3000` yen, `10.125` dollars an hour.
///
/// A string because a JSON number is a float in every browser that reads it,
/// and money is not.
pub fn written(rate: Decimal, currency: &str) -> String {
    // A code the list no longer knows is written exactly as stored, without
    // padding - the one reading of it that cannot be wrong.
    let minor = Currency::from_code(currency).and_then(Currency::exponent).map_or(0, u32::from);
    let mut rate = rate.normalize();
    if rate.scale() < minor {
        rate.rescale(minor);
    }
    rate.to_string()
}

/// Reads a rate written as a decimal string, refusing what the column would
/// change or cannot hold.
pub fn parse_rate(text: &str) -> Result<Decimal, ApiError> {
    let rate = Decimal::from_str_exact(text.trim())
        .map_err(|_| ApiError::bad_request(format!("rate must be a decimal number written as a string, like \"25.50\" - not {text:?}")))?;
    if rate <= Decimal::ZERO {
        return Err(ApiError::bad_request("rate must be more than zero"));
    }
    let rate = rate.normalize();
    if rate.scale() > RATE_SCALE {
        return Err(ApiError::bad_request(format!(
            "rate has more than {RATE_SCALE} decimal places, and would be rounded"
        )));
    }
    if rate.trunc() >= Decimal::from(10_i64.pow(RATE_INTEGER_DIGITS)) {
        return Err(ApiError::bad_request("rate is too large"));
    }
    Ok(rate)
}

/// Reads an ISO 4217 code, refusing one nobody is paid in.
pub fn parse_currency(text: &str) -> Result<Currency, ApiError> {
    let code = text.trim().to_ascii_uppercase();
    let currency = Currency::from_code(&code).ok_or_else(|| ApiError::bad_request(format!("{text:?} is not an ISO 4217 currency code")))?;
    // A retired code is almost always an old habit - kuna after the euro - and
    // would split one person's history across two units for no reason.
    if let Some(successor) = currency.is_superseded() {
        return Err(ApiError::bad_request(format!("{code} has been replaced by {}", successor.latest().code())));
    }
    // Gold, test codes, "no currency": the codes with no minor unit are the
    // ones that are not money anybody earns.
    if currency.is_special() || currency.exponent().is_none() {
        return Err(ApiError::bad_request(format!("{code} is not a currency anybody is paid in")));
    }
    Ok(currency)
}

/// A note as stored: trimmed, and absent rather than blank.
fn clean_note(note: Option<String>) -> Result<Option<String>, ApiError> {
    let Some(note) = note else { return Ok(None) };
    let note = note.trim();
    if note.is_empty() {
        return Ok(None);
    }
    if note.chars().count() > NOTE_MAX_CHARS {
        return Err(ApiError::bad_request(format!("note is longer than {NOTE_MAX_CHARS} characters")));
    }
    Ok(Some(note.to_string()))
}

/// A person's terms, newest first, with where each stands.
#[derive(Debug, Serialize)]
pub struct History {
    pub user_id: Uuid,
    /// The day each `state` was read on: the server's date, like every "today"
    /// on this server (ADR 0003).
    pub as_of: NaiveDate,
    /// Latest start first; of two with one start, the later recorded first.
    pub versions: Vec<VersionView>,
}

/// One version as the API answers it.
#[derive(Debug, Serialize)]
pub struct VersionView {
    pub id: i64,
    pub effective_from: NaiveDate,
    pub basis: Basis,
    /// A decimal string: see [`written`].
    pub rate: String,
    pub currency: String,
    pub note: Option<String>,
    pub state: Standing,
    pub recorded_at: DateTime<Utc>,
    pub recorded_by: Recorder,
}

/// Who entered a version.
#[derive(Debug, Serialize)]
pub struct Recorder {
    pub id: Uuid,
    pub name: String,
}

/// A version to record.
///
/// Unknown fields are refused: an optional field with a typo in its name would
/// otherwise vanish without a word, and on terms that is a note somebody
/// believes was kept.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewVersion {
    pub effective_from: NaiveDate,
    pub basis: Basis,
    /// A decimal string, like `"25.50"`. A JSON number is refused rather than
    /// read through a float.
    pub rate: String,
    pub currency: String,
    #[serde(default)]
    pub note: Option<String>,
}

/// Every version of a person's terms, newest start first.
pub async fn versions_of(pool: &PgPool, user_id: Uuid) -> Result<Vec<Version>, ApiError> {
    let versions = sqlx::query_as(
        "SELECT t.id, t.user_id, t.effective_from, t.basis, t.rate, t.currency, t.note,
                t.recorded_by, r.display_name AS recorded_by_name, t.recorded_at
         FROM pay_terms t
         JOIN users r ON r.id = t.recorded_by
         WHERE t.user_id = $1
         ORDER BY t.effective_from DESC, t.id DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(versions)
}

/// A person's history as the API answers it.
pub async fn history(pool: &PgPool, user_id: Uuid) -> Result<History, ApiError> {
    let versions = versions_of(pool, user_id).await?;
    let today = Norm::today(pool).await?;
    Ok(History {
        user_id,
        as_of: today,
        versions: versions
            .iter()
            .map(|version| VersionView {
                id: version.id,
                effective_from: version.effective_from,
                basis: version.basis,
                rate: written(version.rate, &version.currency),
                currency: version.currency.clone(),
                note: version.note.clone(),
                state: state_of(version, &versions, today),
                recorded_at: version.recorded_at,
                recorded_by: Recorder {
                    id: version.recorded_by,
                    name: version.recorded_by_name.clone(),
                },
            })
            .collect(),
    })
}

/// The account's email, or `404` when there is no such account.
async fn email_of(pool: &PgPool, user_id: Uuid) -> Result<String, ApiError> {
    sqlx::query_scalar("SELECT email FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "no such user"))
}

/// Somebody's terms. Administrators only, until the finance privilege exists;
/// a department's manager sees the days, not the money (ADR 0027).
///
/// Reading another person's terms is recorded. Of everything this server will
/// hold, money is the most sensitive, and "who looked at my pay" has to have
/// an answer.
pub async fn of_user(State(state): State<AppState>, user: CurrentUser, Path(target): Path<Uuid>) -> Result<impl IntoResponse, ApiError> {
    user.require_admin()?;
    let email = email_of(&state.pool, target).await?;
    let history = history(&state.pool, target).await?;

    if target != user.user_id {
        audit::Entry::new(audit::action::PAY_TERMS_VIEWED)
            .by(user.user_id)
            .by_email(&user.email)
            .on(target)
            .labelled(email)
            .record(&state.pool)
            .await;
    }

    Ok(Json(history))
}

/// The person's own terms, with every version and its note.
///
/// `/me` like the person's days: no role and no department are consulted, so
/// there is no permission here to get wrong.
pub async fn mine(State(state): State<AppState>, user: CurrentUser) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(history(&state.pool, user.user_id).await?))
}

/// Records a version of somebody's terms. Administrators only.
///
/// Answers `201` with the person's whole history, because recording one
/// version can change the state of another: the version it corrects becomes
/// `superseded`, and the one it follows `ended`.
pub async fn record(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(target): Path<Uuid>,
    Json(input): Json<NewVersion>,
) -> Result<impl IntoResponse, ApiError> {
    user.require_admin()?;
    let rate = parse_rate(&input.rate)?;
    let currency = parse_currency(&input.currency)?;
    let note = clean_note(input.note)?;
    let email = email_of(&state.pool, target).await?;

    let id: i64 = sqlx::query_scalar(
        "INSERT INTO pay_terms (user_id, effective_from, basis, rate, currency, note, recorded_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id",
    )
    .bind(target)
    .bind(input.effective_from)
    .bind(input.basis)
    .bind(rate)
    .bind(currency.code())
    .bind(note.as_deref())
    .bind(user.user_id)
    .fetch_one(&state.pool)
    .await?;

    let history = history(&state.pool, target).await?;
    // The version this one corrects, if it starts on a date that already had
    // terms: the history says so through `superseded`, the log through this.
    let corrects = history
        .versions
        .iter()
        .filter(|version| version.effective_from == input.effective_from && version.id < id)
        .map(|version| version.id)
        .max();

    tracing::info!(user = %target, version = id, by = %user.user_id, "recorded pay terms");
    // The fact, never the amount. The log is read by administrators, and the
    // finance privilege may separate the two: a rate written here would be
    // read around it (ADR 0027).
    audit::Entry::new(audit::action::PAY_TERMS_RECORDED)
        .by(user.user_id)
        .by_email(&user.email)
        .on(target)
        .labelled(email)
        .with(serde_json::json!({
            "version": id,
            "effective_from": input.effective_from,
            "basis": input.basis,
            "currency": currency.code(),
            "corrects": corrects,
        }))
        .record(&state.pool)
        .await;

    Ok((StatusCode::CREATED, Json(history)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> NaiveDate {
        text.parse().expect("a calendar date")
    }

    fn version(id: i64, effective_from: &str, rate: i64) -> Version {
        Version {
            id,
            user_id: Uuid::nil(),
            effective_from: date(effective_from),
            basis: Basis::Hourly,
            rate: Decimal::from(rate),
            currency: "USD".to_string(),
            note: None,
            recorded_by: Uuid::nil(),
            recorded_by_name: "admin".to_string(),
            recorded_at: DateTime::<Utc>::UNIX_EPOCH,
        }
    }

    fn rate_on(versions: &[Version], day: &str) -> Option<Decimal> {
        in_force(versions, date(day)).map(|version| version.rate)
    }

    #[test]
    fn a_raise_leaves_the_dates_before_it_alone() {
        let versions = [version(1, "2026-01-01", 20), version(2, "2026-03-01", 25)];

        assert_eq!(rate_on(&versions, "2026-02-28"), Some(Decimal::from(20)), "February keeps the old rate");
        assert_eq!(
            rate_on(&versions, "2026-03-01"),
            Some(Decimal::from(25)),
            "the raise applies from its first day"
        );
        assert_eq!(rate_on(&versions, "2026-12-31"), Some(Decimal::from(25)), "and until something replaces it");
    }

    #[test]
    fn before_the_first_version_there_are_no_terms() {
        let versions = [version(1, "2026-01-01", 20)];
        assert_eq!(rate_on(&versions, "2025-12-31"), None, "no terms is not a rate of zero");
        assert_eq!(rate_on(&[], "2026-06-01"), None);
    }

    #[test]
    fn of_two_versions_with_one_start_the_later_recorded_applies() {
        // A mistake and its correction. The order of the slice is not the
        // order of recording: the id is.
        let versions = [version(7, "2026-03-01", 250), version(5, "2026-03-01", 2500)];
        assert_eq!(rate_on(&versions, "2026-03-15"), Some(Decimal::from(250)));

        let reversed = [version(5, "2026-03-01", 2500), version(7, "2026-03-01", 250)];
        assert_eq!(rate_on(&reversed, "2026-03-15"), Some(Decimal::from(250)), "whatever order they are read in");
    }

    #[test]
    fn a_later_start_beats_a_later_record() {
        // A back-dated version entered after a raise: it starts earlier, so it
        // governs only the dates before the raise.
        let versions = [version(1, "2026-03-01", 25), version(2, "2026-01-01", 20)];
        assert_eq!(rate_on(&versions, "2026-02-01"), Some(Decimal::from(20)));
        assert_eq!(
            rate_on(&versions, "2026-04-01"),
            Some(Decimal::from(25)),
            "the raise still holds after its start"
        );
    }

    #[test]
    fn each_version_says_where_it_stands() {
        let versions = [
            version(1, "2026-01-01", 20),
            version(2, "2026-03-01", 2500),
            version(3, "2026-03-01", 25),
            version(4, "2026-12-01", 30),
        ];
        let today = date("2026-10-31");
        let states: Vec<Standing> = versions.iter().map(|version| state_of(version, &versions, today)).collect();
        assert_eq!(states, [Standing::Ended, Standing::Superseded, Standing::Current, Standing::Upcoming]);
    }

    #[test]
    fn a_corrected_version_is_superseded_even_before_it_starts() {
        let versions = [version(1, "2027-01-01", 30), version(2, "2027-01-01", 31)];
        let today = date("2026-10-31");
        assert_eq!(state_of(&versions[0], &versions, today), Standing::Superseded);
        assert_eq!(state_of(&versions[1], &versions, today), Standing::Upcoming);
    }

    #[test]
    fn a_version_starting_today_is_current() {
        let versions = [version(1, "2026-01-01", 20), version(2, "2026-10-31", 25)];
        let today = date("2026-10-31");
        assert_eq!(state_of(&versions[1], &versions, today), Standing::Current);
        assert_eq!(state_of(&versions[0], &versions, today), Standing::Ended);
    }

    #[test]
    fn a_rate_is_a_decimal_string() {
        assert_eq!(parse_rate("25.50").unwrap(), Decimal::new(255, 1));
        assert_eq!(parse_rate(" 3000 ").unwrap(), Decimal::from(3000));
        assert_eq!(parse_rate("10.1250").unwrap(), Decimal::new(10125, 3), "trailing zeros are not places");
        assert!(parse_rate("twenty").is_err());
        assert!(parse_rate("").is_err());
    }

    #[test]
    fn a_rate_the_column_would_change_is_refused() {
        assert!(parse_rate("0").is_err(), "zero is not a rate");
        assert!(parse_rate("-5").is_err(), "nor is a negative");
        assert!(parse_rate("10.12345").is_err(), "a fifth decimal place would be rounded away");
        assert!(parse_rate("10.1234").is_ok(), "four are kept");
        assert!(parse_rate("99999999999999.9999").is_ok(), "the largest the column holds");
        assert!(parse_rate("100000000000000").is_err(), "one more digit does not fit");
    }

    #[test]
    fn a_currency_is_a_current_iso_code() {
        assert_eq!(parse_currency("usd").unwrap().code(), "USD", "case does not matter");
        assert_eq!(parse_currency(" EUR ").unwrap().code(), "EUR");
        assert_eq!(parse_currency("PYG").unwrap().code(), "PYG");
        assert!(parse_currency("ABC").is_err(), "not a code");
        assert!(parse_currency("USDT").is_err(), "not ISO 4217");
        assert!(parse_currency("XAU").is_err(), "gold is not a wage");
        assert!(parse_currency("XXX").is_err(), "nor is no currency");
        let error = parse_currency("HRK").unwrap_err().to_string();
        assert!(error.contains("EUR"), "a retired code names its successor: {error}");
    }

    #[test]
    fn a_rate_is_written_with_its_minor_unit_and_no_more() {
        // As the column returns them: four places, whatever was sent.
        assert_eq!(written(Decimal::new(255_000, 4), "USD"), "25.50");
        assert_eq!(written(Decimal::new(30_000_000, 4), "JPY"), "3000");
        assert_eq!(written(Decimal::new(101_250, 4), "USD"), "10.125");
        assert_eq!(written(Decimal::new(5_000, 4), "BHD"), "0.500", "three places where the currency has three");
        assert_eq!(written(Decimal::new(255_000, 4), "ZZZ"), "25.5", "an unknown code is written exactly");
    }

    #[test]
    fn a_blank_note_is_no_note() {
        assert_eq!(clean_note(None).unwrap(), None);
        assert_eq!(clean_note(Some("   ".to_string())).unwrap(), None);
        assert_eq!(clean_note(Some(" annual review ".to_string())).unwrap().as_deref(), Some("annual review"));
        assert!(clean_note(Some("x".repeat(NOTE_MAX_CHARS + 1))).is_err());
        assert!(clean_note(Some("é".repeat(NOTE_MAX_CHARS))).is_ok(), "the limit counts characters, not bytes");
    }

    #[test]
    fn the_wire_names_are_the_contract() {
        assert_eq!(serde_json::to_value(Basis::Monthly).unwrap(), "monthly");
        assert_eq!(serde_json::to_value(Basis::Hourly).unwrap(), "hourly");
        assert_eq!(serde_json::to_value(Basis::Daily).unwrap(), "daily");
        assert_eq!(serde_json::to_value(Standing::Current).unwrap(), "current");
        assert_eq!(serde_json::to_value(Standing::Upcoming).unwrap(), "upcoming");
        assert_eq!(serde_json::to_value(Standing::Ended).unwrap(), "ended");
        assert_eq!(serde_json::to_value(Standing::Superseded).unwrap(), "superseded");
    }
}
