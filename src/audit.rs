//! The audit log: who did what, to whom, and when.
//!
//! Two properties make it worth a table rather than a log line (ADR 0010).
//! It is queryable - "everything that happened to this person" is a `WHERE`,
//! not a grep across rotated files. And it is part of the data, so it survives
//! wherever the database is backed up to.
//!
//! Writing an entry must never cost a request its work. Every recorded action
//! has already happened by the time it is logged; a failure here is reported
//! loudly and swallowed, because refusing a token revocation because its audit
//! entry would not write is worse in every direction.

use axum::{
    Json,
    extract::{Query, State},
    response::IntoResponse,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{app::AppState, error::ApiError, login::CurrentUser};

/// What happened. Dotted, past tense, `subject.verb`.
///
/// Constants rather than an enum: the set is open - the finance milestone adds
/// its own - and a string that reaches the database unchanged is one less place
/// for a rename to silently reclassify history.
pub mod action {
    pub const USER_CREATED: &str = "user.created";
    pub const USER_UPDATED: &str = "user.updated";
    pub const AGENT_ISSUED: &str = "agent.issued";
    pub const AGENT_REVOKED: &str = "agent.revoked";
    pub const DEPARTMENT_CREATED: &str = "department.created";
    pub const DEPARTMENT_UPDATED: &str = "department.updated";
    pub const DEPARTMENT_DELETED: &str = "department.deleted";
    pub const DEPARTMENT_ASSIGNED: &str = "department.assigned";
    pub const LOGIN_SUCCEEDED: &str = "auth.login";
    pub const LOGIN_FAILED: &str = "auth.login_failed";
    pub const PASSWORD_CHANGED: &str = "auth.password_changed";
    pub const SESSIONS_ENDED: &str = "auth.sessions_ended";
    pub const PRIVACY_LEVEL_CHANGED: &str = "privacy.level_changed";
}

/// One entry being written.
///
/// Built with the chained setters below so a call site reads as a sentence and
/// so adding a field later does not touch every caller.
#[derive(Debug, Default)]
pub struct Entry {
    actor_id: Option<Uuid>,
    actor_email: Option<String>,
    action: String,
    target_id: Option<Uuid>,
    target_label: Option<String>,
    details: Option<serde_json::Value>,
}

impl Entry {
    pub fn new(action: &str) -> Self {
        Self {
            action: action.to_string(),
            ..Default::default()
        }
    }

    /// The person who acted. Absent for the server acting on its own.
    pub fn by(mut self, actor_id: Uuid) -> Self {
        self.actor_id = Some(actor_id);
        self
    }

    /// The actor's email, kept as text so an entry stays readable after a
    /// rename or a deletion.
    pub fn by_email(mut self, email: impl Into<String>) -> Self {
        self.actor_email = Some(email.into());
        self
    }

    pub fn on(mut self, target_id: Uuid) -> Self {
        self.target_id = Some(target_id);
        self
    }

    /// A human label for the target - an email, a department name.
    pub fn labelled(mut self, label: impl Into<String>) -> Self {
        self.target_label = Some(label.into());
        self
    }

    /// Extra context. Never credentials: this is read in an admin UI and
    /// pasted into support tickets.
    pub fn with(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    /// Writes the entry, or complains loudly and carries on.
    ///
    /// The action being recorded has already happened. Failing the request
    /// because its audit entry did not write would undo nothing - the token is
    /// already revoked - and would turn a full disk into an outage.
    pub async fn record(self, pool: &PgPool) {
        let result = sqlx::query(
            "INSERT INTO audit_log (actor_id, actor_email, action, target_id, target_label, details)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(self.actor_id)
        .bind(self.actor_email.as_deref())
        .bind(&self.action)
        .bind(self.target_id)
        .bind(self.target_label.as_deref())
        .bind(self.details.as_ref())
        .execute(pool)
        .await;

        if let Err(error) = result {
            // At error level on purpose: an audit log that stops recording
            // without anyone noticing is worse than one that never existed,
            // because it is trusted.
            tracing::error!(%error, action = %self.action, "failed to write an audit entry");
        }
    }
}

/// An entry as the admin screens read it.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AuditRow {
    pub id: i64,
    pub actor_id: Option<Uuid>,
    pub actor_email: Option<String>,
    pub action: String,
    pub target_id: Option<Uuid>,
    pub target_label: Option<String>,
    pub details: Option<serde_json::Value>,
    pub at: DateTime<Utc>,
}

/// Which slice of the log to read.
#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    /// Everything this person did.
    pub actor_id: Option<Uuid>,
    /// Everything done to this person or thing.
    pub target_id: Option<Uuid>,
    /// One kind of action, e.g. `agent.issued`.
    pub action: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    /// How many entries to return. Clamped; see `MAX_LIMIT`.
    pub limit: Option<i64>,
    /// How many to skip, for paging back through history.
    pub offset: Option<i64>,
}

/// The most entries one request may return.
///
/// A bound rather than a page size an admin can raise: this table grows without
/// limit, and an unbounded read of it is a way to take the server down with a
/// single request.
const MAX_LIMIT: i64 = 500;
const DEFAULT_LIMIT: i64 = 100;

// Neither constant has a unit test. Asserting `DEFAULT_LIMIT <= MAX_LIMIT` here
// would compare two literals and pass at compile time regardless of what the
// handler does with them; the clamp is exercised against the running handler in
// tests/audit.rs, where a request for more than the ceiling must come back with
// exactly the ceiling.

/// Reads the log. Administrators only.
///
/// A manager is deliberately not admitted. The log records who changed what,
/// and until a manager can change anything (ADR 0008) their view of it would
/// consist entirely of other people's actions.
pub async fn list(State(state): State<AppState>, user: CurrentUser, Query(query): Query<AuditQuery>) -> Result<impl IntoResponse, ApiError> {
    user.require_admin()?;

    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let offset = query.offset.unwrap_or(0).max(0);

    let entries: Vec<AuditRow> = sqlx::query_as(
        "SELECT id, actor_id, actor_email, action, target_id, target_label, details, at
         FROM audit_log
         WHERE ($1::uuid IS NULL OR actor_id = $1)
           AND ($2::uuid IS NULL OR target_id = $2)
           AND ($3::text IS NULL OR action = $3)
           AND ($4::timestamptz IS NULL OR at >= $4)
           AND ($5::timestamptz IS NULL OR at <= $5)
         ORDER BY at DESC, id DESC
         LIMIT $6 OFFSET $7",
    )
    .bind(query.actor_id)
    .bind(query.target_id)
    .bind(query.action.as_deref())
    .bind(query.since)
    .bind(query.until)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(entries))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_reads_as_a_sentence() {
        let actor = Uuid::new_v4();
        let target = Uuid::new_v4();
        let entry = Entry::new(action::AGENT_ISSUED)
            .by(actor)
            .by_email("boss@example.test")
            .on(target)
            .labelled("ivan-laptop");

        assert_eq!(entry.action, "agent.issued");
        assert_eq!(entry.actor_id, Some(actor));
        assert_eq!(entry.target_id, Some(target));
        assert_eq!(entry.target_label.as_deref(), Some("ivan-laptop"));
    }

    #[test]
    fn an_entry_without_an_actor_is_allowed() {
        // Provisioning from the environment at startup has no person behind it,
        // and refusing to record it would leave the least explicable changes
        // unrecorded.
        let entry = Entry::new(action::USER_CREATED);
        assert!(entry.actor_id.is_none() && entry.actor_email.is_none());
    }
}
