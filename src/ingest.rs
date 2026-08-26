//! The upload endpoints: `POST /api/v1/days` and `/days/batch`.
//!
//! An agent sends a day (the workday, its pauses, and the tasks recorded on it)
//! and the server writes it whole or not at all. After time offline it sends a
//! stretch of them at once; every day in a batch travels the same path as a
//! live one, and is written on its own so a day that cannot be accepted does
//! not hold up the rest (ADR 0005).
//!
//! Two rules define the contract, both settled before a line of this was
//! written:
//!
//! * **The agent is the source of truth.** A re-upload overwrites what the
//!   server holds for that date. The employee edits their day in kasl - fixes
//!   a task, adds a break they took - and the correction has to land. As a
//!   consequence the same payload sent twice leaves the same rows, which is
//!   what makes a retry after a lost connection safe.
//! * **Timestamps carry an offset, and the day carries its own date.** kasl
//!   stores bare wall-clock text; sending that as-is would make one team's
//!   hours incomparable across time zones. See ADR 0003.

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{
    app::AppState,
    auth::AuthenticatedAgent,
    error::ApiError,
    privacy::{Dropped, Policy, PrivacyLevel},
};

/// One day as the agent recorded it.
#[derive(Debug, Deserialize)]
pub struct DayUpload {
    /// The employee's local calendar date, `YYYY-MM-DD`. Sent explicitly
    /// rather than derived from `started_at`: which day work belongs to is the
    /// agent's call, and near midnight the two disagree.
    pub date: NaiveDate,
    /// When the day started, with the agent's UTC offset.
    pub started_at: DateTime<FixedOffset>,
    /// When it ended; absent while the day is still open.
    #[serde(default)]
    pub ended_at: Option<DateTime<FixedOffset>>,
    #[serde(default)]
    pub pauses: Vec<PauseUpload>,
    #[serde(default)]
    pub tasks: Vec<TaskUpload>,
    /// Whether `tasks` is everything the agent holds for this date.
    ///
    /// When set, a task the server stored on this date and the agent no longer
    /// sends is one the employee deleted, and it is removed here too. Tasks on
    /// other dates are untouched - they are matched by id and outlive a single
    /// day, so wiping by date alone would take yesterday's copy with it.
    ///
    /// Defaults to false: an older agent, which cannot know about this flag,
    /// must never have its silence read as "delete the rest".
    #[serde(default)]
    pub tasks_are_complete: bool,
}

#[derive(Debug, Deserialize)]
pub struct PauseUpload {
    pub started_at: DateTime<FixedOffset>,
    #[serde(default)]
    pub ended_at: Option<DateTime<FixedOffset>>,
    /// Seconds. The agent merges neighbouring pauses before sending, so this
    /// is not always `ended_at - started_at` and is taken as given.
    #[serde(default)]
    pub duration_seconds: Option<i32>,
    /// A break the employee entered by hand (the agent's `protected` flag).
    #[serde(default)]
    pub manual: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TaskUpload {
    /// The agent's own row id. The key a re-upload matches on, so a corrected
    /// task updates instead of piling up.
    pub agent_task_id: i32,
    /// The agent's `task_id`: the same work carried across several days.
    /// Defaults to `agent_task_id`, which is what the agent stores for a task
    /// started today.
    #[serde(default)]
    pub agent_group_id: Option<i32>,
    pub recorded_at: DateTime<FixedOffset>,
    pub name: String,
    #[serde(default)]
    pub comment: Option<String>,
    /// Percent complete, 0..=100.
    pub completeness: i16,
}

/// What the agent gets back: enough to log, and to notice a silent no-op.
#[derive(Debug, Serialize)]
pub struct DayAccepted {
    pub workday_id: Uuid,
    pub date: NaiveDate,
    pub pauses: usize,
    pub tasks: usize,
    /// Tasks dropped because the agent declared its set authoritative. Zero on
    /// the common upload; a non-zero count is worth noticing in a log.
    pub deleted_tasks: u64,
    /// The privacy level that applied. Always reported, even at `full`: an
    /// agent should be able to tell an installation that keeps everything from
    /// one whose policy it has not read yet.
    pub privacy_level: PrivacyLevel,
    /// What that level discarded. Absent from the response when it discarded
    /// nothing, which is the common case (ADR 0011).
    #[serde(skip_serializing_if = "Dropped::is_empty")]
    pub discarded: Dropped,
}

/// A stretch of days at once - what an agent sends after time offline.
#[derive(Debug, Deserialize)]
pub struct BatchUpload {
    pub days: Vec<DayUpload>,
}

/// What came of a batch. Counts first: a caller that reads only the status sees
/// `200` even when a day was refused, so the summary has to be impossible to
/// miss in the body.
#[derive(Debug, Serialize)]
pub struct BatchResult {
    pub accepted: usize,
    pub rejected: usize,
    pub results: Vec<DayResult>,
}

/// One day's fate, in the order the days were sent.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum DayResult {
    Accepted {
        #[serde(flatten)]
        day: DayAccepted,
    },
    /// The date is echoed even here: it is how the agent knows which of its
    /// pending days to keep, and a day can be refused before anything else
    /// about it is known to be usable.
    Rejected { date: NaiveDate, error: String },
}

/// Accepts one day from an authenticated agent.
pub async fn upload_day(State(state): State<AppState>, agent: AuthenticatedAgent, Json(day): Json<DayUpload>) -> Result<impl IntoResponse, ApiError> {
    let policy = Policy::load(&state.pool).await?;
    let accepted = store_day(&state.pool, agent, &day, policy).await?;
    Ok((StatusCode::OK, Json(accepted)))
}

/// Accepts a backlog of days, each written on its own.
///
/// One bad day does not sink the batch. An agent holding a day the server will
/// never accept would otherwise be unable to deliver any of its backlog, and
/// would retry the same doomed request forever (ADR 0005).
pub async fn upload_batch(State(state): State<AppState>, agent: AuthenticatedAgent, Json(batch): Json<BatchUpload>) -> Result<impl IntoResponse, ApiError> {
    if batch.days.len() > state.max_batch_days {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("a batch carries at most {} days; split the backlog", state.max_batch_days),
        ));
    }

    // Once for the batch, not once per day in it: thirty days of backfill
    // are one policy, and reading it thirty times would only add ways for the
    // days in one request to disagree with each other.
    let policy = Policy::load(&state.pool).await?;

    let mut results = Vec::with_capacity(batch.days.len());
    let mut accepted = 0;
    let mut rejected = 0;

    for day in &batch.days {
        match store_day(&state.pool, agent, day, policy).await {
            Ok(stored) => {
                accepted += 1;
                results.push(DayResult::Accepted { day: stored });
            }
            // A day the server itself failed on aborts the batch: the agent
            // must retry it, and reporting a database outage as "this day is
            // rejected" would tell it to give up instead.
            Err(error) if error.status().is_server_error() => return Err(error),
            Err(error) => {
                rejected += 1;
                results.push(DayResult::Rejected {
                    date: day.date,
                    error: error.to_string(),
                });
            }
        }
    }

    tracing::info!(user_id = %agent.user_id, agent_id = %agent.agent_id, accepted, rejected, "accepted a batch");

    Ok((StatusCode::OK, Json(BatchResult { accepted, rejected, results })))
}

/// Writes one day, whole or not at all.
///
/// Shared by the single-day route and the batch one so a backfilled day is
/// stored by exactly the same code as a live one.
async fn store_day(pool: &sqlx::PgPool, agent: AuthenticatedAgent, day: &DayUpload, policy: Policy) -> Result<DayAccepted, ApiError> {
    validate(day)?;

    // Before the transaction, deliberately: what the level excludes is never
    // handed to a statement, so it cannot be written and then filtered on the
    // way out. The promise is about the disk (ADR 0011).
    let level = policy.level();
    let (day, discarded, pause_totals) = filter(day, level);
    let day = &day;

    // All of it or none: a day whose pauses landed but whose tasks did not
    // would show up on a dashboard as real, and nobody would know to re-send.
    let mut tx = pool.begin().await?;

    let workday_id = upsert_workday(&mut tx, agent.user_id, day, pause_totals).await?;
    replace_pauses(&mut tx, workday_id, &day.pauses).await?;
    let tasks = upsert_tasks(&mut tx, agent.user_id, day.date, &day.tasks).await?;
    // A level that stores no tasks clears the date's, flag or no flag. Without
    // this, a task written while the policy was wider survives a re-upload
    // under a narrower one, and the server keeps a name and a comment the
    // policy says it does not hold - found by driving the real thing, because
    // the pauses next to it are replaced wholesale and looked fine.
    let deleted_tasks = if day.tasks_are_complete || !level.keeps_tasks() {
        delete_missing_tasks(&mut tx, agent.user_id, day.date, &day.tasks).await?
    } else {
        0
    };

    tx.commit().await?;

    // The agent, not just the person: several machines report for one employee
    // and "which one sent this" is the first question when a day looks wrong.
    tracing::info!(%workday_id, user_id = %agent.user_id, agent_id = %agent.agent_id, date = %day.date, pauses = day.pauses.len(), tasks, deleted_tasks, "accepted a day");

    Ok(DayAccepted {
        workday_id,
        date: day.date,
        pauses: day.pauses.len(),
        tasks,
        deleted_tasks,
        privacy_level: level,
        discarded,
    })
}

/// Applies the installation's privacy level to a day before anything is
/// written.
///
/// Returns the day as it will be stored, what was left out, and - for a level
/// that does not keep pauses one by one - what they came to. The totals are
/// computed here, from what the agent sent, because after filtering the rows
/// are gone and the day could no longer describe itself.
///
/// The day is rebuilt rather than mutated in place because the caller holds a
/// borrowed upload that a batch will reuse: filtering must not change what the
/// next day in the request sees.
fn filter(day: &DayUpload, level: PrivacyLevel) -> (DayUpload, Dropped, Option<PauseTotals>) {
    let mut discarded = Dropped::default();

    let pauses = if level.keeps_pause_times() {
        day.pauses
            .iter()
            .map(|pause| PauseUpload {
                started_at: pause.started_at,
                ended_at: pause.ended_at,
                duration_seconds: pause.duration_seconds,
                manual: pause.manual,
                reason: match (&pause.reason, level.keeps_free_text()) {
                    (Some(reason), false) if !reason.is_empty() => {
                        discarded.free_text += 1;
                        None
                    }
                    (reason, true) => reason.clone(),
                    _ => None,
                },
            })
            .collect()
    } else {
        // Not stored one by one - the day carries the count and the total
        // instead, so its hours still add up (see `paused_count` in the
        // schema). An empty list here means "no rows", not "no interruptions".
        discarded.pauses = day.pauses.len();
        Vec::new()
    };

    let tasks = if level.keeps_tasks() {
        day.tasks
            .iter()
            .map(|task| TaskUpload {
                agent_task_id: task.agent_task_id,
                agent_group_id: task.agent_group_id,
                recorded_at: task.recorded_at,
                name: task.name.clone(),
                comment: match (&task.comment, level.keeps_free_text()) {
                    (Some(comment), false) if !comment.is_empty() => {
                        discarded.free_text += 1;
                        None
                    }
                    (comment, true) => comment.clone(),
                    _ => None,
                },
                completeness: task.completeness,
            })
            .collect()
    } else {
        discarded.tasks = day.tasks.len();
        Vec::new()
    };

    // `tasks_are_complete` is carried through even where no tasks are stored:
    // it is how a level that stops keeping tasks clears the ones an earlier,
    // wider level left behind. Tightening the policy does not erase history on
    // its own, but a day the agent re-sends is stored under the policy in
    // force now.
    let filtered = DayUpload {
        date: day.date,
        started_at: day.started_at,
        ended_at: day.ended_at,
        pauses,
        tasks,
        tasks_are_complete: day.tasks_are_complete,
    };

    let totals = (!level.keeps_pause_times()).then(|| pause_totals(&day.pauses));

    (filtered, discarded, totals)
}

/// What a day's pauses came to: how many, and how long in total.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PauseTotals {
    count: i32,
    seconds: i32,
}

/// What a day's pauses come to, for the levels that do not store them one by
/// one.
///
/// Counted from what the agent sent, before filtering drops the rows: the
/// summary has to describe the day that happened, not the empty list left
/// after the policy is applied.
fn pause_totals(pauses: &[PauseUpload]) -> PauseTotals {
    let count = pauses.len() as i32;
    let seconds = pauses
        .iter()
        .map(|pause| {
            pause.duration_seconds.unwrap_or_else(|| {
                // A pause the agent did not measure: fall back to the interval
                // it reported, and to zero for one still running, which has no
                // duration yet by definition.
                pause
                    .ended_at
                    .map(|ended| (ended - pause.started_at).num_seconds().clamp(0, i32::MAX as i64) as i32)
                    .unwrap_or(0)
            })
        })
        .fold(0i32, |total, seconds| total.saturating_add(seconds));

    PauseTotals { count, seconds }
}

/// Rejects payloads the schema would refuse anyway, with a message that says
/// which field is wrong - a constraint violation surfaces as a 500 and tells
/// the agent nothing it can act on.
fn validate(day: &DayUpload) -> Result<(), ApiError> {
    if let Some(ended_at) = day.ended_at
        && ended_at < day.started_at
    {
        return Err(ApiError::bad_request("ended_at is before started_at"));
    }

    for (index, pause) in day.pauses.iter().enumerate() {
        if let Some(ended_at) = pause.ended_at
            && ended_at < pause.started_at
        {
            return Err(ApiError::bad_request(format!("pauses[{index}]: ended_at is before started_at")));
        }
        if pause.duration_seconds.is_some_and(|seconds| seconds < 0) {
            return Err(ApiError::bad_request(format!("pauses[{index}]: duration_seconds is negative")));
        }
    }

    for (index, task) in day.tasks.iter().enumerate() {
        if !(0..=100).contains(&task.completeness) {
            return Err(ApiError::bad_request(format!("tasks[{index}]: completeness must be between 0 and 100")));
        }
        if task.name.trim().is_empty() {
            return Err(ApiError::bad_request(format!("tasks[{index}]: name is empty")));
        }
    }

    Ok(())
}

/// Writes the day, or corrects the one already stored for that date.
///
/// `pause_totals` is set only under a level that does not store pauses one by
/// one. Without it such a day would claim uninterrupted work - a more
/// flattering picture than the truth, and a false one.
async fn upsert_workday(tx: &mut Transaction<'_, Postgres>, user_id: Uuid, day: &DayUpload, pause_totals: Option<PauseTotals>) -> Result<Uuid, ApiError> {
    let workday_id: Uuid = sqlx::query_scalar(
        "INSERT INTO workdays (user_id, date, started_at, ended_at, paused_count, paused_seconds) VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (user_id, date) DO UPDATE SET
             started_at = EXCLUDED.started_at,
             ended_at = EXCLUDED.ended_at,
             paused_count = EXCLUDED.paused_count,
             paused_seconds = EXCLUDED.paused_seconds
         RETURNING id",
    )
    .bind(user_id)
    .bind(day.date)
    .bind(day.started_at.with_timezone(&Utc))
    .bind(day.ended_at.map(|at| at.with_timezone(&Utc)))
    .bind(pause_totals.map(|totals| totals.count))
    .bind(pause_totals.map(|totals| totals.seconds))
    .fetch_one(&mut **tx)
    .await?;

    Ok(workday_id)
}

/// Replaces the day's pauses wholesale.
///
/// Pauses have no agent-side identity to match on - the agent splits and
/// merges them as activity comes in - so the day's set is what was sent, and
/// a pause the employee deleted disappears here too.
async fn replace_pauses(tx: &mut Transaction<'_, Postgres>, workday_id: Uuid, pauses: &[PauseUpload]) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM pauses WHERE workday_id = $1")
        .bind(workday_id)
        .execute(&mut **tx)
        .await?;

    for pause in pauses {
        sqlx::query("INSERT INTO pauses (workday_id, started_at, ended_at, duration_seconds, manual, reason) VALUES ($1, $2, $3, $4, $5, $6)")
            .bind(workday_id)
            .bind(pause.started_at.with_timezone(&Utc))
            .bind(pause.ended_at.map(|at| at.with_timezone(&Utc)))
            .bind(pause.duration_seconds)
            .bind(pause.manual)
            .bind(pause.reason.as_deref())
            .execute(&mut **tx)
            .await?;
    }

    Ok(())
}

/// Writes the day's tasks, correcting any the agent has sent before.
///
/// Tasks do carry an agent-side id, so they are matched rather than replaced:
/// the same task may appear on several days, and wiping by date would take
/// yesterday's copy with it.
async fn upsert_tasks(tx: &mut Transaction<'_, Postgres>, user_id: Uuid, date: NaiveDate, tasks: &[TaskUpload]) -> Result<usize, ApiError> {
    for task in tasks {
        sqlx::query(
            "INSERT INTO tasks (user_id, agent_task_id, agent_group_id, date, recorded_at, name, comment, completeness)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (user_id, agent_task_id) DO UPDATE SET
                 agent_group_id = EXCLUDED.agent_group_id,
                 date = EXCLUDED.date,
                 recorded_at = EXCLUDED.recorded_at,
                 name = EXCLUDED.name,
                 comment = EXCLUDED.comment,
                 completeness = EXCLUDED.completeness",
        )
        .bind(user_id)
        .bind(task.agent_task_id)
        .bind(task.agent_group_id.unwrap_or(task.agent_task_id))
        .bind(date)
        .bind(task.recorded_at.with_timezone(&Utc))
        .bind(task.name.trim())
        .bind(task.comment.as_deref())
        .bind(task.completeness)
        .execute(&mut **tx)
        .await?;
    }

    Ok(tasks.len())
}

/// Removes the date's tasks the agent did not send.
///
/// Only reached when the agent marked its list authoritative. Scoped to the
/// one date on purpose: a task carried across several days keeps its rows on
/// the others, and an agent backfilling Monday cannot erase Friday.
async fn delete_missing_tasks(tx: &mut Transaction<'_, Postgres>, user_id: Uuid, date: NaiveDate, tasks: &[TaskUpload]) -> Result<u64, ApiError> {
    let kept: Vec<i32> = tasks.iter().map(|task| task.agent_task_id).collect();

    let deleted = sqlx::query("DELETE FROM tasks WHERE user_id = $1 AND date = $2 AND agent_task_id <> ALL($3)")
        .bind(user_id)
        .bind(date)
        .bind(&kept)
        .execute(&mut **tx)
        .await?
        .rows_affected();

    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day_json(patch: serde_json::Value) -> DayUpload {
        let mut value = serde_json::json!({
            "date": "2026-08-14",
            "started_at": "2026-08-14T09:00:00-03:00",
            "pauses": [],
            "tasks": [],
        });
        let (serde_json::Value::Object(base), serde_json::Value::Object(patch)) = (&mut value, patch) else {
            panic!("both must be objects");
        };
        base.extend(patch);
        serde_json::from_value(value).expect("the fixture should deserialize")
    }

    #[test]
    fn an_offset_is_required_on_every_instant() {
        // The whole point of the contract: bare wall-clock time, which is what
        // kasl stores locally, must not parse.
        let bare = serde_json::json!({
            "date": "2026-08-14",
            "started_at": "2026-08-14T09:00:00",
        });
        assert!(
            serde_json::from_value::<DayUpload>(bare).is_err(),
            "an instant without an offset must be rejected"
        );
    }

    #[test]
    fn the_offset_is_preserved_as_an_instant() {
        let day = day_json(serde_json::json!({ "started_at": "2026-08-14T09:00:00-03:00" }));
        assert_eq!(day.started_at.with_timezone(&Utc).to_rfc3339(), "2026-08-14T12:00:00+00:00");
    }

    #[test]
    fn a_day_may_still_be_open() {
        let day = day_json(serde_json::json!({}));
        assert!(day.ended_at.is_none(), "a missing ended_at means the day is still running");
        validate(&day).expect("an open day is valid");
    }

    #[test]
    fn a_day_cannot_end_before_it_starts() {
        let day = day_json(serde_json::json!({ "ended_at": "2026-08-14T08:00:00-03:00" }));
        let error = validate(&day).expect_err("a backwards day must be refused");
        assert_eq!(error.to_string(), "ended_at is before started_at");
    }

    #[test]
    fn impossible_pauses_and_tasks_are_named_in_the_error() {
        let day = day_json(serde_json::json!({
            "pauses": [
                {"started_at": "2026-08-14T10:00:00-03:00", "ended_at": "2026-08-14T10:20:00-03:00", "duration_seconds": 1200},
                {"started_at": "2026-08-14T12:00:00-03:00", "duration_seconds": -1},
            ],
        }));
        let error = validate(&day).expect_err("a negative duration must be refused");
        assert!(
            error.to_string().contains("pauses[1]"),
            "the message should point at the offending element: {error}"
        );

        let day = day_json(serde_json::json!({
            "tasks": [{"agent_task_id": 1, "recorded_at": "2026-08-14T17:00:00-03:00", "name": "Ship it", "completeness": 101}],
        }));
        let error = validate(&day).expect_err("completeness above 100 must be refused");
        assert!(
            error.to_string().contains("tasks[0]"),
            "the message should point at the offending element: {error}"
        );
    }

    #[test]
    fn a_task_group_defaults_to_the_task_itself() {
        let day = day_json(serde_json::json!({
            "tasks": [{"agent_task_id": 7, "recorded_at": "2026-08-14T17:00:00-03:00", "name": "Write the ingest", "completeness": 60}],
        }));
        let task = &day.tasks[0];
        assert_eq!(task.agent_group_id, None, "an absent group is absent on the wire");
        assert_eq!(task.agent_group_id.unwrap_or(task.agent_task_id), 7, "and resolves to the task itself");
    }

    #[test]
    fn an_agent_that_says_nothing_deletes_nothing() {
        // The compatibility hinge: agents shipped before this flag existed send
        // whatever tasks they have, and their silence must not be read as
        // "delete everything else on that date".
        let day = day_json(serde_json::json!({}));
        assert!(!day.tasks_are_complete, "the authoritative set must be opt-in");

        let day = day_json(serde_json::json!({ "tasks_are_complete": true }));
        assert!(day.tasks_are_complete, "and an agent that opts in is heard");
    }

    #[test]
    fn a_nameless_task_is_refused() {
        let day = day_json(serde_json::json!({
            "tasks": [{"agent_task_id": 1, "recorded_at": "2026-08-14T17:00:00-03:00", "name": "   ", "completeness": 50}],
        }));
        assert!(validate(&day).is_err(), "a task with a blank name carries no information");
    }
}
