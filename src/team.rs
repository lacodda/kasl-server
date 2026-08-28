//! What a manager may read about other people: `GET /api/v1/team/days` and
//! `GET /api/v1/users/{id}/days`.
//!
//! This is where the server starts answering for someone other than the caller,
//! so the permission is the subject of this module rather than a detail inside
//! it. Two rules, both settled before the code:
//!
//! * **The visibility rule lives in one place.** [`admin::VISIBLE_USERS`] is
//!   pasted into every query here. A second copy of "who may see whom" is a
//!   second chance for one of them to widen, and a leak of this kind is
//!   invisible to the person leaked about.
//! * **A summary, not a pile of days.** The dashboard shows a row per person;
//!   the timeline of one day belongs to the drill-down, which reuses the shape
//!   `/me/days` already answers. A single endpoint carrying every pause of
//!   twenty people for a month would ship a payload nobody on that screen
//!   reads.
//!
//! A person the reader may see is listed **even with nothing recorded**. The
//! employee whose agent has never reported is exactly who a manager needs to
//! notice, and dropping them from the table hides the case the dashboard exists
//! for - the same "emptiness lies by default" defect the privacy work fixed at
//! ingest (ADR 0011).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    admin::{VISIBLE_USERS, require_manager_or_admin},
    app::AppState,
    error::ApiError,
    login::CurrentUser,
    me::{self, Range},
    model::UserRole,
    privacy::Policy,
};

/// One person's period, as the dashboard's table shows it.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Member {
    pub id: Uuid,
    pub display_name: String,
    pub email: String,
    pub department: Option<String>,
    /// Days with a workday row in the range. Zero is a real answer.
    pub days_recorded: i64,
    /// Seconds worked across the range: the span of each finished day less
    /// what was paused in it. Open days contribute nothing - a day still
    /// running has no total to add (the same rule `/me/days` follows).
    pub worked_seconds: i64,
    pub paused_seconds: i64,
    /// The most recent date with a workday, so "no data" can be told from
    /// "nothing since the 12th".
    pub last_day: Option<NaiveDate>,
    /// Whether a day is open right now on the employee's own calendar.
    pub day_open: bool,
    /// When any of this person's agents last delivered anything.
    ///
    /// The honest half of "who is working now": the server knows when it last
    /// heard from a machine, not whether someone is at it. Live status needs
    /// heartbeats, which is its own milestone.
    pub last_seen_at: Option<DateTime<Utc>>,
    /// Live agent tokens. Zero explains a silent row without guessing.
    pub agents: i64,
}

/// The team's period.
#[derive(Debug, Serialize)]
pub struct Team {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub members: Vec<Member>,
    /// The level in force, so the dashboard can caveat its own figures the way
    /// the personal page does.
    pub privacy_level: crate::privacy::PrivacyLevel,
    pub not_stored: Vec<&'static str>,
}

/// Answers the team's hours over a range.
pub async fn days(State(state): State<AppState>, user: CurrentUser, Query(range): Query<Range>) -> Result<impl IntoResponse, ApiError> {
    require_manager_or_admin(&user)?;
    me::validate_range(&range)?;

    let is_admin = user.role == UserRole::Admin;

    // `date` here is the employee's own local date, as their agent recorded it,
    // and "today" is the server's. They can differ by a day at the edges; for
    // "is a day open" that is the right approximation - the alternative needs a
    // per-person time zone the server does not store (ADR 0003).
    // `AssertSqlSafe` because the only interpolation is `VISIBLE_USERS`, a
    // constant in `admin`; every value from the request is bound below.
    let members: Vec<Member> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT u.id, u.display_name, u.email, d.name AS department,
                coalesce(w.days_recorded, 0)::bigint AS days_recorded,
                coalesce(w.worked_seconds, 0)::bigint AS worked_seconds,
                coalesce(w.paused_seconds, 0)::bigint AS paused_seconds,
                w.last_day,
                coalesce(o.day_open, false) AS day_open,
                (SELECT max(a.last_seen_at) FROM agents a WHERE a.user_id = u.id) AS last_seen_at,
                (SELECT count(*) FROM agents a WHERE a.user_id = u.id AND a.revoked_at IS NULL) AS agents
         FROM users u
         LEFT JOIN departments d ON d.id = u.department_id
         LEFT JOIN LATERAL (
             -- `sum()` over bigint answers `numeric`, which does not decode
             -- into an i64; the cast is outside the sum so it happens once.
             SELECT count(*) AS days_recorded,
                    max(w.date) AS last_day,
                    coalesce(sum(
                        CASE WHEN w.ended_at IS NULL THEN 0
                             ELSE greatest(extract(epoch FROM (w.ended_at - w.started_at))::bigint - paused.seconds, 0)
                        END
                    ), 0)::bigint AS worked_seconds,
                    coalesce(sum(paused.seconds), 0)::bigint AS paused_seconds
             FROM workdays w
             CROSS JOIN LATERAL (
                 -- Stored pauses where they exist; the day's own totals where a
                 -- narrower policy summarized them away (ADR 0011). One or the
                 -- other, never both, so the hours cannot be double-counted.
                 SELECT CASE
                     WHEN EXISTS (SELECT 1 FROM pauses p WHERE p.workday_id = w.id)
                     THEN (SELECT coalesce(sum(p.duration_seconds), 0)::bigint FROM pauses p WHERE p.workday_id = w.id)
                     ELSE coalesce(w.paused_seconds, 0)::bigint
                 END AS seconds
             ) AS paused
             WHERE w.user_id = u.id AND w.date BETWEEN $3 AND $4
         ) AS w ON true
         LEFT JOIN LATERAL (
             SELECT true AS day_open
             FROM workdays w2
             WHERE w2.user_id = u.id AND w2.date = current_date AND w2.ended_at IS NULL
             LIMIT 1
         ) AS o ON true
         WHERE u.active AND {VISIBLE_USERS}
         ORDER BY u.display_name, u.email"
    )))
    .bind(is_admin)
    .bind(user.user_id)
    .bind(range.from)
    .bind(range.to)
    .fetch_all(&state.pool)
    .await?;

    let level = Policy::load(&state.pool).await?.level();

    Ok(Json(Team {
        from: range.from,
        to: range.to,
        members,
        privacy_level: level,
        not_stored: me::not_stored_at(level),
    }))
}

/// Answers one person's days to someone allowed to see them.
///
/// Deliberately the same response shape as `/me/days`: the drill-down is the
/// personal screen pointed at someone else, and two shapes for one thing would
/// mean two renderers to keep in step.
pub async fn user_days(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(target): Path<Uuid>,
    Query(range): Query<Range>,
) -> Result<impl IntoResponse, ApiError> {
    require_manager_or_admin(&user)?;
    me::validate_range(&range)?;

    if !may_read(&state.pool, &user, target).await? {
        // Not "no such user": a manager probing ids should not be able to tell
        // an employee in another department from one who does not exist.
        return Err(ApiError::new(StatusCode::NOT_FOUND, "no such user"));
    }

    Ok(Json(me::days_for(&state.pool, target, &range).await?))
}

/// Whether `reader` may see `target`'s data.
///
/// Asked of the database with the same clause the listing uses, rather than
/// reasoned about in Rust: the rule and the check cannot drift if they are the
/// same string.
async fn may_read(pool: &PgPool, reader: &CurrentUser, target: Uuid) -> Result<bool, ApiError> {
    let visible: Option<Uuid> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT u.id FROM users u WHERE u.id = $3 AND {VISIBLE_USERS}")))
        .bind(reader.role == UserRole::Admin)
        .bind(reader.user_id)
        .bind(target)
        .fetch_optional(pool)
        .await?;

    Ok(visible.is_some())
}
