//! The agent's pulse: `POST /api/v1/agent/heartbeat`, and the live status the
//! dashboard reads off it.
//!
//! Everything before this milestone answered about the past. A day arrives
//! after it is over - or, for an open day, once in a while - and the manager's
//! table said "last data 2 h ago", which is honest and not what the screen is
//! for. The pulse is the one thing an agent can say about *now*: kasl is
//! running on this machine, and the person it reports for is working, on a
//! break, or not in a day at all.
//!
//! Three rules define it, all settled before the code:
//!
//! * **The agent claims the state; the server times it.** kasl knows whether
//!   its watcher is inside a pause - the server would have to infer it from
//!   rows that arrive minutes later. But staleness is measured against the
//!   server's clock: a machine whose clock is a day off would otherwise look
//!   permanently offline, or permanently alive.
//! * **A missing pulse is not a state.** An agent too old to know this route,
//!   or one that has been switched off, has no state at all, and the dashboard
//!   says "unknown" rather than inventing `idle`. Reading silence as a claim
//!   is the same defect the privacy work fixed at ingest: emptiness must not
//!   lie (ADR 0011).
//! * **The pulse says no more than the state.** Not the task, not the reason
//!   for the break - those live in the day, under the privacy level that
//!   governs them. A live feed of what someone is doing this minute is a
//!   different product from a time tracker, and this one is not it (ADR 0014).

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use chrono::{DateTime, Duration, FixedOffset, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{app::AppState, auth::AuthenticatedAgent, error::ApiError};

/// How often an agent is asked to report in.
///
/// Sent back on every pulse rather than configured in the agent: the interval
/// and the staleness threshold below have to agree, and only one side can own
/// that. An agent that guessed its own would eventually guess something the
/// server calls offline.
pub const INTERVAL_SECONDS: i64 = 60;

/// How long a pulse stays believable, in seconds.
///
/// Three intervals: two missed pulses are a slow network or a laptop lid, and
/// calling that "offline" would make the dashboard flicker at people who are
/// working. The third miss is a real absence.
pub const STALE_AFTER_SECONDS: i64 = INTERVAL_SECONDS * 3;

/// How far ahead of the server an agent's own clock may be before its stamp is
/// refused.
///
/// A stamp from the future would sit "fresh" for as long as the skew lasts,
/// which is exactly how a stuck agent could look alive for a day. A minute of
/// tolerance covers ordinary clock drift; beyond that the agent is told, so a
/// person can fix the clock rather than trust a dashboard that is lying.
const MAX_CLOCK_SKEW_SECONDS: i64 = 60;

/// What an agent claims its person is doing. Mirrors the `agent_state` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "agent_state", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum AgentState {
    /// In a working day, and the watcher sees activity.
    Working,
    /// In a working day, inside a pause - a detected idle stretch or a break
    /// the employee entered by hand.
    Paused,
    /// Running and reporting, but not in a working day: before the day starts,
    /// after it is closed, on a weekend.
    Idle,
}

/// What an agent sends.
#[derive(Debug, Deserialize)]
pub struct Pulse {
    pub state: AgentState,
    /// When the agent observed this, with its own UTC offset - the same
    /// convention every timestamp in this API follows (ADR 0003).
    ///
    /// Sent rather than left to the server so a pulse delayed by a slow link
    /// is not read as a fresher observation than it is.
    pub at: DateTime<FixedOffset>,
}

/// What the agent is told back.
///
/// The agent gets to see the server's reading of its own pulse: kasl can show
/// "the server thinks you are offline" in the CLI, where the employee already
/// is, instead of leaving them to discover it on a dashboard they cannot see.
#[derive(Debug, Serialize)]
pub struct Accepted {
    /// Seconds until the agent should report again.
    pub interval_seconds: i64,
    /// After how many seconds of silence the server stops believing a pulse.
    pub stale_after_seconds: i64,
    /// The state as recorded, echoed so a mismatch is visible at the agent.
    pub state: AgentState,
    /// How far the agent's clock is from the server's, in seconds, positive
    /// when the agent is ahead. Reported rather than silently corrected: a
    /// clock that is minutes out makes every hour this server stores wrong,
    /// and only the machine's owner can fix it.
    pub clock_skew_seconds: i64,
}

/// Records a pulse.
pub async fn beat(State(state): State<AppState>, agent: AuthenticatedAgent, Json(pulse): Json<Pulse>) -> Result<impl IntoResponse, ApiError> {
    let now = Utc::now();
    let at = pulse.at.with_timezone(&Utc);
    let skew = (at - now).num_seconds();

    if skew > MAX_CLOCK_SKEW_SECONDS {
        // Refused rather than clamped. Clamping would store a plausible stamp
        // and leave the real fault - a machine whose clock is wrong, and whose
        // uploaded days are therefore wrong too - invisible at both ends.
        return Err(ApiError::bad_request(format!(
            "the pulse is stamped {skew} s ahead of this server; check the machine's clock"
        )));
    }

    sqlx::query("UPDATE agents SET heartbeat_state = $2, heartbeat_at = $3, heartbeat_received_at = now() WHERE id = $1")
        .bind(agent.agent_id)
        .bind(pulse.state)
        .bind(at)
        .execute(&state.pool)
        .await?;

    Ok((
        StatusCode::ACCEPTED,
        Json(Accepted {
            interval_seconds: INTERVAL_SECONDS,
            stale_after_seconds: STALE_AFTER_SECONDS,
            state: pulse.state,
            clock_skew_seconds: skew,
        }),
    ))
}

/// What the dashboard shows for one person, right now.
///
/// Deliberately wider than [`AgentState`]: `offline` and `unknown` are not
/// things an agent can claim, they are what the server concludes from silence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LiveStatus {
    Working,
    Paused,
    Idle,
    /// A pulse was received once, but not recently enough to believe.
    Offline,
    /// No pulse has ever arrived: no agent, an agent switched off since before
    /// this server started asking, or a kasl too old to know the route.
    Unknown,
}

impl LiveStatus {
    /// Resolves a stored pulse into what the dashboard shows.
    ///
    /// The one place the threshold is applied. `received` is the server's own
    /// stamp on purpose - see the module note on clocks.
    pub fn resolve(state: Option<AgentState>, received: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Self {
        let (Some(state), Some(received)) = (state, received) else {
            return Self::Unknown;
        };
        if now - received > Duration::seconds(STALE_AFTER_SECONDS) {
            return Self::Offline;
        }
        match state {
            AgentState::Working => Self::Working,
            AgentState::Paused => Self::Paused,
            AgentState::Idle => Self::Idle,
        }
    }
}

/// One person's live row.
#[derive(Debug, Serialize)]
pub struct Live {
    pub user_id: Uuid,
    /// What the dashboard should show, with the staleness threshold applied.
    pub status: LiveStatus,
    /// Seconds since the server received that pulse. `None` when there is
    /// none - the dashboard prints "unknown", not "0 seconds ago".
    pub since_received: Option<i64>,
}

/// Loads the live status of everyone the reader may see.
///
/// Takes the visibility clause from the caller rather than embedding one:
/// there is exactly one rule for who may see whom ([`crate::admin::VISIBLE_USERS`]),
/// and a second copy of it here would be a second chance for one of them to
/// widen.
pub async fn load(pool: &PgPool, visible_users: &str, is_admin: bool, reader: Uuid) -> Result<Vec<Live>, ApiError> {
    // `AssertSqlSafe` because the only interpolation is `visible_users`, a
    // constant in `admin`; every value from the request is bound.
    let rows: Vec<(Uuid, Option<AgentState>, Option<DateTime<Utc>>)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT u.id, h.heartbeat_state, h.heartbeat_received_at
         FROM users u
         LEFT JOIN LATERAL (
             -- The freshest pulse among this person's live agents. Someone
             -- with a desktop and a laptop is working if either says so, and
             -- a revoked agent's last words are not evidence of anything.
             SELECT a.heartbeat_state, a.heartbeat_received_at
             FROM agents a
             WHERE a.user_id = u.id AND a.revoked_at IS NULL AND a.heartbeat_received_at IS NOT NULL
             ORDER BY a.heartbeat_received_at DESC
             LIMIT 1
         ) AS h ON true
         WHERE u.active AND {visible_users}
         ORDER BY u.display_name, u.email"
    )))
    .bind(is_admin)
    .bind(reader)
    .fetch_all(pool)
    .await?;

    // The threshold is applied here rather than in SQL so `resolve` is the
    // single definition of "offline" that the unit tests can reach.
    let now = Utc::now();
    Ok(rows
        .into_iter()
        .map(|(user_id, state, received)| Live {
            user_id,
            status: LiveStatus::resolve(state, received, now),
            since_received: received.map(|received| (now - received).num_seconds().max(0)),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seconds_ago(now: DateTime<Utc>, seconds: i64) -> Option<DateTime<Utc>> {
        Some(now - Duration::seconds(seconds))
    }

    #[test]
    fn the_wire_names_are_the_contract() {
        // Renaming any of these is a breaking API change for kasl agents and
        // for the web UI, so it has to fail here rather than at a puzzled
        // client (the same guard `model` keeps over roles).
        assert_eq!(serde_json::to_string(&AgentState::Working).unwrap(), r#""working""#);
        assert_eq!(serde_json::to_string(&AgentState::Paused).unwrap(), r#""paused""#);
        assert_eq!(serde_json::to_string(&AgentState::Idle).unwrap(), r#""idle""#);
        assert_eq!(serde_json::to_string(&LiveStatus::Offline).unwrap(), r#""offline""#);
        assert_eq!(serde_json::to_string(&LiveStatus::Unknown).unwrap(), r#""unknown""#);
    }

    #[test]
    fn a_state_round_trips_through_json() {
        for state in [AgentState::Working, AgentState::Paused, AgentState::Idle] {
            let json = serde_json::to_string(&state).unwrap();
            assert_eq!(serde_json::from_str::<AgentState>(&json).unwrap(), state);
        }
    }

    #[test]
    fn a_fresh_pulse_is_shown_as_claimed() {
        let now = Utc::now();
        for (state, expected) in [
            (AgentState::Working, LiveStatus::Working),
            (AgentState::Paused, LiveStatus::Paused),
            (AgentState::Idle, LiveStatus::Idle),
        ] {
            assert_eq!(LiveStatus::resolve(Some(state), seconds_ago(now, 5), now), expected);
        }
    }

    #[test]
    fn a_pulse_survives_two_missed_intervals() {
        // The reason the threshold is three intervals and not one: a laptop
        // lid or a slow link must not paint someone who is working as gone.
        let now = Utc::now();
        let two_missed = INTERVAL_SECONDS * 2 + 5;
        assert_eq!(
            LiveStatus::resolve(Some(AgentState::Working), seconds_ago(now, two_missed), now),
            LiveStatus::Working
        );
    }

    #[test]
    fn a_stale_pulse_is_offline_whatever_it_claimed() {
        // The defect this guards: showing the last claim forever. An agent
        // killed mid-day would leave "working" on the dashboard indefinitely,
        // which is worse than no status at all - it is a wrong one.
        let now = Utc::now();
        let stale = seconds_ago(now, STALE_AFTER_SECONDS + 1);
        for state in [AgentState::Working, AgentState::Paused, AgentState::Idle] {
            assert_eq!(LiveStatus::resolve(Some(state), stale, now), LiveStatus::Offline);
        }
    }

    #[test]
    fn silence_is_unknown_rather_than_idle() {
        // An agent that never sent a pulse and an agent that says "not in a
        // day" are different facts, and only one of them is evidence about
        // the person. Collapsing them would tell a manager that everyone
        // running an older kasl has stopped working.
        let now = Utc::now();
        assert_eq!(LiveStatus::resolve(None, None, now), LiveStatus::Unknown);
        // A state without a receipt cannot be aged, so it is not believable
        // either - this pairing should be impossible in the schema, and if it
        // ever happens it must not read as a live claim.
        assert_eq!(LiveStatus::resolve(Some(AgentState::Working), None, now), LiveStatus::Unknown);
        assert_eq!(LiveStatus::resolve(None, seconds_ago(now, 1), now), LiveStatus::Unknown);
    }

    #[test]
    fn the_interval_leaves_room_for_a_missed_pulse() {
        // The agent is told both numbers and must be able to miss one without
        // being called offline; a threshold at or below the interval would
        // make that impossible however punctual the agent is.
        const { assert!(STALE_AFTER_SECONDS > INTERVAL_SECONDS, "an agent gets no margin at all") };
    }
}
