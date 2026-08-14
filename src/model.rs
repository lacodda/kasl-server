//! The domain the server stores: the shapes behind `migrations/`.
//!
//! These types are the Rust half of the schema. They exist now, before the
//! ingest API that fills them, so the tables have one authoritative reading -
//! column, type and meaning together - instead of a SQL file plus whatever a
//! handler happens to select.
//!
//! Naming follows the database, and the database follows kasl: a reader who
//! knows the agent's model recognizes this one.
//!
//! Nothing reads these structs yet - the ingest API and the queries behind it
//! are the next milestones. They are allowed to sit unused rather than be
//! written twice: the schema and its Rust reading land together, and the
//! serialization test below already holds the wire names to the contract.
#![allow(dead_code)]

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// What a user may do. Mirrors the `user_role` enum in PostgreSQL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "user_role", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    /// Runs the installation: accounts, agents, server settings.
    Admin,
    /// Sees the team (or their department) and its reports.
    Manager,
    /// Sees themselves.
    Employee,
}

/// A person in the installation.
///
/// `password_hash` is deliberately absent: nothing outside authentication has
/// a reason to load a verifier, and a struct that never holds one cannot leak
/// it into a response.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub role: UserRole,
    /// Deactivated accounts keep their history and stop being accepted.
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// One installed kasl reporting on a user's behalf.
///
/// The token itself lives only in the agent's config; the server keeps a hash.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Agent {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    /// Set when the token is withdrawn; the row and its data stay.
    pub revoked_at: Option<DateTime<Utc>>,
    /// Last accepted request, for "this agent went silent" signals.
    pub last_seen_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A working day of one person.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Workday {
    pub id: Uuid,
    pub user_id: Uuid,
    /// The employee's local calendar date, as the agent recorded it - not a
    /// date derived from `started_at` in whatever zone the server runs.
    pub date: NaiveDate,
    pub started_at: DateTime<Utc>,
    /// `None` while the day is still open.
    pub ended_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// An interruption inside a workday: detected idleness or a manual break.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Pause {
    pub id: Uuid,
    pub workday_id: Uuid,
    pub started_at: DateTime<Utc>,
    /// `None` while the pause is still running on the agent.
    pub ended_at: Option<DateTime<Utc>>,
    /// Seconds. Stored rather than derived: the agent merges neighbouring
    /// pauses across a gap, so the duration is not always end minus start.
    pub duration_seconds: Option<i32>,
    /// Entered by the employee (the agent's `protected` flag): never merged
    /// away and exempt from the short-pause thresholds.
    pub manual: bool,
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A task the employee logged for a day.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Task {
    pub id: Uuid,
    pub user_id: Uuid,
    /// The agent's own row id, unique per user: the key a re-upload matches on.
    pub agent_task_id: i32,
    /// The agent's `task_id`, tying the same work carried across several days.
    pub agent_group_id: i32,
    /// The employee's local date the task belongs to.
    pub date: NaiveDate,
    pub recorded_at: DateTime<Utc>,
    pub name: String,
    pub comment: Option<String>,
    /// Percent complete, 0..=100.
    pub completeness: i16,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A label on tasks. Scoped to one user: vocabularies are personal.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Tag {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub color: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Which period a report covers. Mirrors the `report_kind` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "report_kind", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum ReportKind {
    Daily,
    Monthly,
}

/// The event of a report being submitted, with the figures as of that moment.
///
/// Not a second copy of the day: hours are recomputed from workdays and pauses
/// whenever they are shown. What cannot be recomputed is what the employee
/// actually submitted and when - which is what approval rests on.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Report {
    pub id: Uuid,
    pub user_id: Uuid,
    pub kind: ReportKind,
    /// The day itself for a daily report, the first of the month for a monthly.
    pub period_start: NaiveDate,
    pub submitted_at: DateTime<Utc>,
    pub worked_seconds: i32,
    /// Percent, as the agent computes productivity.
    pub productivity: Option<f32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire names are part of the contract with kasl agents and the web UI:
    /// a rename here is a breaking API change, so it should fail a test, not
    /// surface as a puzzled client.
    #[test]
    fn roles_and_report_kinds_serialize_in_lowercase() {
        assert_eq!(serde_json::to_string(&UserRole::Admin).unwrap(), r#""admin""#);
        assert_eq!(serde_json::to_string(&UserRole::Manager).unwrap(), r#""manager""#);
        assert_eq!(serde_json::to_string(&UserRole::Employee).unwrap(), r#""employee""#);
        assert_eq!(serde_json::to_string(&ReportKind::Daily).unwrap(), r#""daily""#);
        assert_eq!(serde_json::to_string(&ReportKind::Monthly).unwrap(), r#""monthly""#);
    }

    #[test]
    fn roles_round_trip_through_json() {
        for role in [UserRole::Admin, UserRole::Manager, UserRole::Employee] {
            let json = serde_json::to_string(&role).unwrap();
            assert_eq!(serde_json::from_str::<UserRole>(&json).unwrap(), role);
        }
    }
}
