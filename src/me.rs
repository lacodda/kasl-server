//! What a person can read about themselves: `GET /api/v1/me/days`.
//!
//! The first read endpoint this server has. Every route before it either took
//! data in or described the installation; this one hands a day back, and its
//! shape is reused by the manager's drill-down in [`crate::team`] - the same
//! answer for a different subject, through [`days_for`].
//!
//! Two decisions stand behind it, both taken before the code:
//!
//! * **A range, not a day.** The screen draws a week, the calendar a month,
//!   and a drill-down one date - all three are `from`/`to` with different
//!   ends. An endpoint per screen would make the API a description of the
//!   current UI rather than a contract.
//! * **`/me`, not `/users/{id}` with your own id.** Nothing here consults a
//!   role or a department, so no reading of them can be wrong. Someone else's
//!   days go through [`crate::team`], where the permission is the subject and
//!   is checked in the open.
//!
//! The response says what the privacy level withheld, rather than answering an
//! empty list. Under `coarse` the server stores no individual pauses, and a
//! screen that draws nothing there would tell the employee they worked without
//! a break - the reassuring reading, and the false one (ADR 0011).

use axum::{
    Json,
    extract::{Query, State},
    response::IntoResponse,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    app::AppState,
    error::ApiError,
    login::CurrentUser,
    privacy::{Policy, PrivacyLevel},
};

/// The widest range one request may ask for, in days.
///
/// A year and a bit: enough for "my whole history" on the screens that offer
/// it, and bounded so a hand-written query cannot ask the server to serialize
/// an installation's lifetime in one response.
pub const MAX_RANGE_DAYS: i64 = 400;

/// The period being asked for. Both ends inclusive - `from=2026-08-27` and
/// `to=2026-08-27` is one day, which is what a person writing the query by
/// hand expects.
#[derive(Debug, Deserialize)]
pub struct Range {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

/// One day, with everything stored under it.
///
/// Field names follow `DayUpload` where they mean the same thing: an agent
/// author reading both ends of the API should not have to learn two
/// vocabularies for one day.
#[derive(Debug, Serialize)]
pub struct Day {
    pub date: NaiveDate,
    pub started_at: DateTime<Utc>,
    /// Absent while the day is still open on the agent.
    pub ended_at: Option<DateTime<Utc>>,
    /// Seconds between start and end, minus what was paused. `None` for a day
    /// that has not ended: a half-finished day has no total, and reporting the
    /// hours so far as the day's figure would make an open day look short.
    pub worked_seconds: Option<i64>,
    /// How many times the day was interrupted, and for how long in total.
    /// Always answered - computed from the stored pauses where they exist, and
    /// read off the day where the policy summarized them away.
    pub paused_count: i64,
    pub paused_seconds: i64,
    pub pauses: Vec<Pause>,
    pub tasks: Vec<Task>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Pause {
    pub id: Uuid,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub duration_seconds: Option<i32>,
    /// A break the employee entered by hand, as opposed to detected idleness.
    pub manual: bool,
    /// The text they typed with it, where the policy keeps free text.
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Task {
    pub id: Uuid,
    pub name: String,
    pub comment: Option<String>,
    pub completeness: i16,
    pub recorded_at: DateTime<Utc>,
}

/// The answer: the days, and what the installation's policy left out of them.
#[derive(Debug, Serialize)]
pub struct Days {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub days: Vec<Day>,
    /// The level in force now. Not necessarily the one these days were stored
    /// under - narrowing does not rewrite history (ADR 0011) - which is why
    /// the omissions below are stated per kind rather than inferred from it.
    pub privacy_level: PrivacyLevel,
    /// Kinds of detail this installation does not keep, named so a screen can
    /// say "not stored" where it would otherwise show an empty section. An
    /// empty list means nothing is withheld.
    pub not_stored: Vec<&'static str>,
}

/// What a level withholds, in the words a screen can show as-is.
pub fn not_stored_at(level: PrivacyLevel) -> Vec<&'static str> {
    let mut withheld = Vec::new();
    if !level.keeps_pause_times() {
        withheld.push("pauses");
    }
    if !level.keeps_tasks() {
        withheld.push("tasks");
    }
    if !level.keeps_free_text() {
        withheld.push("free_text");
    }
    withheld
}

/// Answers the signed-in person's own days.
pub async fn days(State(state): State<AppState>, user: CurrentUser, Query(range): Query<Range>) -> Result<impl IntoResponse, ApiError> {
    validate_range(&range)?;
    Ok(Json(days_for(&state.pool, user.user_id, &range).await?))
}

/// Builds one person's answer, whoever is asking.
///
/// Shared with the manager's drill-down, which is this screen pointed at
/// someone else: the permission differs, the answer must not. Callers check who
/// may read what before they get here.
pub async fn days_for(pool: &PgPool, user_id: Uuid, range: &Range) -> Result<Days, ApiError> {
    let level = Policy::load(pool).await?.level();
    let days = load_days(pool, user_id, range).await?;

    Ok(Days {
        from: range.from,
        to: range.to,
        days,
        privacy_level: level,
        not_stored: not_stored_at(level),
    })
}

/// Rejects a range the server will not serve, with the reason.
///
/// Separate from the handler so the rules can be read - and tested - without a
/// database behind them.
pub fn validate_range(range: &Range) -> Result<(), ApiError> {
    if range.to < range.from {
        return Err(ApiError::bad_request("`to` is before `from`"));
    }
    // Inclusive on both ends, so a single day is a span of zero.
    let span = (range.to - range.from).num_days() + 1;
    if span > MAX_RANGE_DAYS {
        return Err(ApiError::bad_request(format!(
            "a range covers at most {MAX_RANGE_DAYS} days, this one covers {span}"
        )));
    }
    Ok(())
}

/// Loads the days in a range with their pauses and tasks.
///
/// Three queries rather than one join: a day joined to both its pauses and its
/// tasks multiplies the rows, and reassembling that in Rust is where an hour
/// gets counted twice. Each query is bounded by the same range, which the
/// handler has already capped.
async fn load_days(pool: &PgPool, user_id: Uuid, range: &Range) -> Result<Vec<Day>, ApiError> {
    let workdays: Vec<WorkdayRow> = sqlx::query_as(
        r#"
        SELECT id, date, started_at, ended_at, paused_count, paused_seconds
        FROM workdays
        WHERE user_id = $1 AND date BETWEEN $2 AND $3
        ORDER BY date
        "#,
    )
    .bind(user_id)
    .bind(range.from)
    .bind(range.to)
    .fetch_all(pool)
    .await?;

    if workdays.is_empty() {
        return Ok(Vec::new());
    }

    let workday_ids: Vec<Uuid> = workdays.iter().map(|day| day.id).collect();

    let pauses: Vec<(Uuid, Pause)> = sqlx::query_as::<_, PauseRow>(
        r#"
        SELECT workday_id, id, started_at, ended_at, duration_seconds, manual, reason
        FROM pauses
        WHERE workday_id = ANY($1)
        ORDER BY started_at
        "#,
    )
    .bind(&workday_ids)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(PauseRow::split)
    .collect();

    // Tasks hang off the user and a date, not off the workday: kasl carries the
    // same task across days, and a task can be logged on a date whose workday
    // never arrived.
    let tasks: Vec<(NaiveDate, Task)> = sqlx::query_as::<_, TaskRow>(
        r#"
        SELECT date, id, name, comment, completeness, recorded_at
        FROM tasks
        WHERE user_id = $1 AND date BETWEEN $2 AND $3
        ORDER BY recorded_at
        "#,
    )
    .bind(user_id)
    .bind(range.from)
    .bind(range.to)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(TaskRow::split)
    .collect();

    Ok(workdays.into_iter().map(|row| row.into_day(&pauses, &tasks)).collect())
}

/// A workday as stored, before its pauses and tasks are attached.
#[derive(Debug, sqlx::FromRow)]
struct WorkdayRow {
    id: Uuid,
    date: NaiveDate,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
    paused_count: Option<i32>,
    paused_seconds: Option<i32>,
}

impl WorkdayRow {
    fn into_day(self, pauses: &[(Uuid, Pause)], tasks: &[(NaiveDate, Task)]) -> Day {
        let own_pauses: Vec<Pause> = pauses.iter().filter(|(id, _)| *id == self.id).map(|(_, pause)| pause.clone_row()).collect();
        let own_tasks: Vec<Task> = tasks.iter().filter(|(date, _)| *date == self.date).map(|(_, task)| task.clone_row()).collect();

        // Two sources for one figure, and only one of them exists at a time.
        // Where pauses are stored, they are the count; where the policy
        // summarized them away, the day carries what they came to (ADR 0011).
        // Preferring the stored rows keeps the number consistent with the
        // timeline drawn next to it.
        let (paused_count, paused_seconds) = if own_pauses.is_empty() && (self.paused_count.is_some() || self.paused_seconds.is_some()) {
            (i64::from(self.paused_count.unwrap_or(0)), i64::from(self.paused_seconds.unwrap_or(0)))
        } else {
            let seconds = own_pauses.iter().filter_map(|pause| pause.duration_seconds).map(i64::from).sum();
            (own_pauses.len() as i64, seconds)
        };

        let worked_seconds = self.ended_at.map(|ended| ((ended - self.started_at).num_seconds() - paused_seconds).max(0));

        Day {
            date: self.date,
            started_at: self.started_at,
            ended_at: self.ended_at,
            worked_seconds,
            paused_count,
            paused_seconds,
            pauses: own_pauses,
            tasks: own_tasks,
        }
    }
}

/// A pause with the workday it belongs to, for grouping.
#[derive(Debug, sqlx::FromRow)]
struct PauseRow {
    workday_id: Uuid,
    id: Uuid,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
    duration_seconds: Option<i32>,
    manual: bool,
    reason: Option<String>,
}

impl PauseRow {
    fn split(self) -> (Uuid, Pause) {
        (
            self.workday_id,
            Pause {
                id: self.id,
                started_at: self.started_at,
                ended_at: self.ended_at,
                duration_seconds: self.duration_seconds,
                manual: self.manual,
                reason: self.reason,
            },
        )
    }
}

impl Pause {
    fn clone_row(&self) -> Self {
        Self {
            id: self.id,
            started_at: self.started_at,
            ended_at: self.ended_at,
            duration_seconds: self.duration_seconds,
            manual: self.manual,
            reason: self.reason.clone(),
        }
    }
}

/// A task with the date it belongs to, for grouping.
#[derive(Debug, sqlx::FromRow)]
struct TaskRow {
    date: NaiveDate,
    id: Uuid,
    name: String,
    comment: Option<String>,
    completeness: i16,
    recorded_at: DateTime<Utc>,
}

impl TaskRow {
    fn split(self) -> (NaiveDate, Task) {
        (
            self.date,
            Task {
                id: self.id,
                name: self.name,
                comment: self.comment,
                completeness: self.completeness,
                recorded_at: self.recorded_at,
            },
        )
    }
}

impl Task {
    fn clone_row(&self) -> Self {
        Self {
            id: self.id,
            name: self.name.clone(),
            comment: self.comment.clone(),
            completeness: self.completeness,
            recorded_at: self.recorded_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(from: &str, to: &str) -> Range {
        Range {
            from: from.parse().expect("a test date"),
            to: to.parse().expect("a test date"),
        }
    }

    #[test]
    fn a_single_day_is_a_valid_range() {
        // Both ends inclusive: the drill-down asks for one date with the same
        // parameter twice, and rejecting that would make the common case the
        // awkward one.
        assert!(validate_range(&range("2026-08-27", "2026-08-27")).is_ok());
    }

    #[test]
    fn a_backwards_range_is_refused() {
        let error = validate_range(&range("2026-08-27", "2026-08-01")).unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);
        assert!(error.to_string().contains("before"), "the message should say what is wrong: {error}");
    }

    #[test]
    fn the_range_has_a_ceiling() {
        // The boundary itself, on both sides: an off-by-one here either
        // refuses a legitimate year or lifts the cap the test claims to guard.
        let widest = range("2026-01-01", "2027-02-04");
        assert_eq!((widest.to - widest.from).num_days() + 1, MAX_RANGE_DAYS);
        assert!(validate_range(&widest).is_ok(), "exactly {MAX_RANGE_DAYS} days is allowed");

        let too_wide = range("2026-01-01", "2027-02-05");
        let error = validate_range(&too_wide).unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);
        assert!(error.to_string().contains("401"), "the message should name the span asked for: {error}");
    }

    #[test]
    fn a_level_names_what_it_withholds() {
        // What the screen renders as "the server does not store this". Under
        // `full` the list is empty, and an empty section then really does mean
        // nothing happened.
        assert!(not_stored_at(PrivacyLevel::Full).is_empty());
        assert_eq!(not_stored_at(PrivacyLevel::Moderate), vec!["free_text"]);
        assert_eq!(not_stored_at(PrivacyLevel::Coarse), vec!["pauses", "tasks", "free_text"]);
    }
}
