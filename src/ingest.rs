//! The upload endpoint: `POST /api/v1/days`.
//!
//! An agent sends one day at a time - the workday, its pauses, and the tasks
//! recorded on it - and the server writes it whole or not at all.
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

use crate::{app::AppState, auth::AuthenticatedAgent, error::ApiError};

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
}

/// Accepts one day from an authenticated agent.
pub async fn upload_day(State(state): State<AppState>, agent: AuthenticatedAgent, Json(day): Json<DayUpload>) -> Result<impl IntoResponse, ApiError> {
    validate(&day)?;

    // All of it or none: a day whose pauses landed but whose tasks did not
    // would show up on a dashboard as real, and nobody would know to re-send.
    let mut tx = state.pool.begin().await?;

    let workday_id = upsert_workday(&mut tx, agent.user_id, &day).await?;
    replace_pauses(&mut tx, workday_id, &day.pauses).await?;
    let tasks = upsert_tasks(&mut tx, agent.user_id, day.date, &day.tasks).await?;

    tx.commit().await?;

    // The agent, not just the person: several machines report for one employee
    // and "which one sent this" is the first question when a day looks wrong.
    tracing::info!(%workday_id, user_id = %agent.user_id, agent_id = %agent.agent_id, date = %day.date, pauses = day.pauses.len(), tasks, "accepted a day");

    Ok((
        StatusCode::OK,
        Json(DayAccepted {
            workday_id,
            date: day.date,
            pauses: day.pauses.len(),
            tasks,
        }),
    ))
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
async fn upsert_workday(tx: &mut Transaction<'_, Postgres>, user_id: Uuid, day: &DayUpload) -> Result<Uuid, ApiError> {
    let workday_id: Uuid = sqlx::query_scalar(
        "INSERT INTO workdays (user_id, date, started_at, ended_at) VALUES ($1, $2, $3, $4)
         ON CONFLICT (user_id, date) DO UPDATE SET started_at = EXCLUDED.started_at, ended_at = EXCLUDED.ended_at
         RETURNING id",
    )
    .bind(user_id)
    .bind(day.date)
    .bind(day.started_at.with_timezone(&Utc))
    .bind(day.ended_at.map(|at| at.with_timezone(&Utc)))
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
    fn a_nameless_task_is_refused() {
        let day = day_json(serde_json::json!({
            "tasks": [{"agent_task_id": 1, "recorded_at": "2026-08-14T17:00:00-03:00", "name": "   ", "completeness": 50}],
        }));
        assert!(validate(&day).is_err(), "a task with a blank name carries no information");
    }
}
