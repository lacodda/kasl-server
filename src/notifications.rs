//! Notifications: the server tells the person a fact is about.
//!
//! Everything said before this milestone was said to a manager - on the
//! dashboard (ADR 0018), in a team's chat (ADR 0019). The person an alert is
//! about heard nothing, including about the one alert they can fix: a day the
//! server has held open for seventeen hours is usually a close kasl made on
//! the laptop that never arrived, and the only person able to send it again
//! was the only person not told.
//!
//! **A notification is a message to one person, and it is stored.** Written in
//! the transaction that made its fact true, so the fact and the telling commit
//! together - the same rule the webhooks follow, for the same reason: a notice
//! held in memory is lost on the restart that happens at the wrong moment.
//!
//! Two readers, and one text for both (ADR 0020):
//!
//! * **kasl**, which cannot be reached - laptops sleep behind NAT - and so asks.
//!   The pulse it sends every minute is answered with how many notices this
//!   machine has not shown; it reads them, shows them as toasts, and says how
//!   far it got. The cursor is kept here, per machine, so a reinstall does not
//!   replay a year and a second machine gets its own toast rather than none.
//! * **the web inbox**, which is the person's own record of what they were
//!   told, read up to a point like any list of messages.
//!
//! The server writes the sentence. An agent older than a kind shows the words
//! and is right; one that knows the kind can act on the fields beside them.

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::{
    alerts::AlertRule,
    app::AppState,
    auth::AuthenticatedAgent,
    error::ApiError,
    login::CurrentUser,
    privacy::PrivacyLevel,
    webhooks::{AlertPayload, Webhooks, hours, span},
};

/// What a notification is about. Mirrors the `notification_kind` enum.
///
/// Each is a fact the employee cannot see from where they are. Acknowledging
/// an alert is deliberately not among them: "your manager decided it was fine"
/// is a comfort rather than something to act on, and a channel that says
/// everything teaches people to ignore it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "notification_kind")]
pub enum NotificationKind {
    /// The server raised an alert about you, and your manager was told.
    #[serde(rename = "alert.raised")]
    #[sqlx(rename = "alert.raised")]
    AlertRaised,
    /// A new token can report as you.
    #[serde(rename = "agent.issued")]
    #[sqlx(rename = "agent.issued")]
    AgentIssued,
    /// A machine can no longer report as you.
    #[serde(rename = "agent.revoked")]
    #[sqlx(rename = "agent.revoked")]
    AgentRevoked,
    /// What this server keeps about your days changed.
    #[serde(rename = "privacy.changed")]
    #[sqlx(rename = "privacy.changed")]
    PrivacyChanged,
}

/// The machine a notice is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentFact {
    pub id: Uuid,
    /// Its name as it was when the person was told. A rename later does not
    /// change a notice already read.
    pub name: String,
}

/// A change of privacy level, as both ends of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivacyFact {
    pub from: PrivacyLevel,
    pub to: PrivacyLevel,
}

/// The facts a notice was made from, stored as its `payload`.
///
/// One field is set, the one its kind names. A struct of options rather than
/// an enum because this is also the response's shape, flattened next to the
/// words: an agent reads `alert.subject_date` whatever else it does not know.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Facts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alert: Option<AlertPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentFact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privacy: Option<PrivacyFact>,
}

// Writing -----------------------------------------------------------------------
//
// `sqlx::Error` rather than `ApiError`: these run inside handlers and inside
// the startup provisioning alike, and each caller already knows how to turn a
// database error into its own.

/// Tells a person that an alert about them was raised.
///
/// In the sweep's transaction. `ON CONFLICT (alert_id)`: one alert is told
/// once, and two sweeps racing to raise it would otherwise tell it twice.
pub async fn alert_raised(conn: &mut PgConnection, user_id: Uuid, alert: &AlertPayload) -> Result<(), sqlx::Error> {
    let facts = Facts {
        alert: Some(alert.clone()),
        ..Facts::default()
    };
    sqlx::query(
        "INSERT INTO notifications (user_id, kind, alert_id, payload) VALUES ($1, 'alert.raised', $2, $3)
         ON CONFLICT (alert_id) DO NOTHING",
    )
    .bind(user_id)
    .bind(alert.id)
    .bind(sqlx::types::Json(&facts))
    .execute(conn)
    .await?;
    Ok(())
}

/// Tells a person that a machine can now report as them, or no longer can.
///
/// Wherever a token is issued or withdrawn - the admin screen and the
/// environment's `KASL_AGENTS` alike. The reason for the notice is that the
/// person may not know, and that is as true of a token an operator seeded as
/// of one an administrator clicked.
pub async fn agent_changed(conn: &mut PgConnection, kind: NotificationKind, user_id: Uuid, agent_id: Uuid, name: &str) -> Result<(), sqlx::Error> {
    debug_assert!(matches!(kind, NotificationKind::AgentIssued | NotificationKind::AgentRevoked));
    let facts = Facts {
        agent: Some(AgentFact {
            id: agent_id,
            name: name.to_string(),
        }),
        ..Facts::default()
    };
    sqlx::query("INSERT INTO notifications (user_id, kind, agent_id, payload) VALUES ($1, $2, $3, $4)")
        .bind(user_id)
        .bind(kind)
        .bind(agent_id)
        .bind(sqlx::types::Json(&facts))
        .execute(conn)
        .await?;
    Ok(())
}

/// Tells everybody active that what the server keeps about them changed.
///
/// Everybody, because the level is the installation's and not a person's
/// (ADR 0011); active, because a deactivated account has nobody left to read
/// it. Returns how many were told.
pub async fn privacy_changed(conn: &mut PgConnection, from: PrivacyLevel, to: PrivacyLevel) -> Result<u64, sqlx::Error> {
    let facts = Facts {
        privacy: Some(PrivacyFact { from, to }),
        ..Facts::default()
    };
    let written = sqlx::query("INSERT INTO notifications (user_id, kind, payload) SELECT id, 'privacy.changed', $1 FROM users WHERE active")
        .bind(sqlx::types::Json(&facts))
        .execute(conn)
        .await?;
    Ok(written.rows_affected())
}

// Reading -------------------------------------------------------------------------

/// One notification, as both the agent and the inbox receive it.
#[derive(Debug, Serialize)]
pub struct Notification {
    /// The order it was said in, and what a reader acknowledges up to.
    pub id: i64,
    pub kind: NotificationKind,
    pub created_at: DateTime<Utc>,
    /// The sentence, written here so every reader says the same one - and an
    /// agent that has never heard of `kind` still has something true to show.
    pub title: String,
    pub body: String,
    /// When what it said stopped being true: the alert it announced resolved.
    /// A notice that is over is kept, and shown as over, rather than removed -
    /// "your manager was told on Monday" stays a fact after Tuesday fixes it.
    pub withdrawn_at: Option<DateTime<Utc>>,
    /// Whether the person has seen it.
    pub read: bool,
    /// Where to look in the web UI, when the installation knows its address
    /// (`KASL_PUBLIC_URL`). For a toast; the inbox routes itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(flatten)]
    pub facts: Facts,
}

#[derive(Debug, sqlx::FromRow)]
struct Row {
    id: i64,
    kind: NotificationKind,
    created_at: DateTime<Utc>,
    payload: sqlx::types::Json<Facts>,
    withdrawn_at: Option<DateTime<Utc>>,
    read: bool,
}

impl Row {
    fn into_notification(self, webhooks: &Webhooks) -> Notification {
        let facts = self.payload.0;
        let (title, body) = words(self.id, self.kind, &facts);
        Notification {
            id: self.id,
            kind: self.kind,
            created_at: self.created_at,
            title,
            body,
            withdrawn_at: self.withdrawn_at,
            read: self.read,
            link: webhooks.link(where_to_look(self.kind)),
            facts,
        }
    }
}

/// The screen that shows what a notice is about.
fn where_to_look(kind: NotificationKind) -> &'static str {
    match kind {
        NotificationKind::AlertRaised => "/day",
        NotificationKind::PrivacyChanged => "/privacy",
        NotificationKind::AgentIssued | NotificationKind::AgentRevoked => "/notifications",
    }
}

/// What a notice says, as a title and a sentence.
///
/// Every sentence states what was measured and what it was measured against,
/// in the dashboard's units - the rule the alerts and the webhooks follow
/// (ADR 0016). None of them says what it means: an eleven-hour day is a
/// release, a crisis or kasl left running, and the server knows no more than
/// that it happened.
pub fn words(id: i64, kind: NotificationKind, facts: &Facts) -> (String, String) {
    match (kind, facts) {
        (NotificationKind::AlertRaised, Facts { alert: Some(alert), .. }) => alert_words(alert),
        (NotificationKind::AgentIssued, Facts { agent: Some(agent), .. }) => (
            format!("\u{201c}{}\u{201d} can now report as you", agent.name),
            "A token by that name was issued for your account. Its days will be filed under your name. If this is not your machine, tell an administrator of this server.".to_string(),
        ),
        (NotificationKind::AgentRevoked, Facts { agent: Some(agent), .. }) => (
            format!("\u{201c}{}\u{201d} can no longer report as you", agent.name),
            "Its token was revoked. What it already sent stays; anything it sends from now on is refused.".to_string(),
        ),
        (NotificationKind::PrivacyChanged, Facts { privacy: Some(privacy), .. }) => (
            "What this server keeps about you changed".to_string(),
            format!(
                "The privacy level went from {} to {}. {} It applies to what arrives from now on.",
                level_name(privacy.from),
                level_name(privacy.to),
                crate::privacy::summary_for(privacy.to)
            ),
        ),
        // A row whose payload does not carry what its kind names. Nothing in
        // this module writes one, and a restore of a hand-edited backup could.
        // Said plainly rather than rendered as an empty toast, and logged so
        // the operator can find the row.
        (kind, _) => {
            tracing::warn!(id, ?kind, "a notification is missing the facts its kind needs");
            ("A notice from kasl-server".to_string(), "Open the web UI to read it.".to_string())
        }
    }
}

fn alert_words(alert: &AlertPayload) -> (String, String) {
    let date = alert.subject_date.map(|date| date.to_string()).unwrap_or_default();
    match alert.rule {
        AlertRule::DayNotClosed => (
            format!("Your day of {date} is still open here"),
            format!(
                "This server has had it open for {}, and your manager was told. If kasl closed the day on your machine, the close did not arrive: `kasl server push --date {date}` sends it again.",
                span(alert.observed_seconds)
            ),
        ),
        AlertRule::Overwork => (
            format!("Your manager was told about {date}"),
            format!(
                "You worked {} h that day, against your norm of {} h.",
                hours(alert.observed_seconds),
                alert.against_seconds.map(hours).unwrap_or_else(|| "—".to_string())
            ),
        ),
        AlertRule::NoAgentData => (
            "Your manager was told your machines went quiet".to_string(),
            format!("Nothing had arrived from any of your machines for {}.", span(alert.observed_seconds)),
        ),
    }
}

fn level_name(level: PrivacyLevel) -> &'static str {
    match level {
        PrivacyLevel::Full => "full",
        PrivacyLevel::Moderate => "moderate",
        PrivacyLevel::Coarse => "coarse",
    }
}

/// What an agent has not shown yet: the condition, for the list and the count.
///
/// `$1` is the agent. Each line is a rule of ADR 0020:
///
/// * past this machine's cursor, and past what the person has already read -
///   something seen in the inbox is not toasted anywhere afterwards;
/// * said after this machine was issued, so a token handed out today does not
///   replay what happened before it existed;
/// * not about this machine itself - "laptop can now report as you" is news to
///   the desktop, and noise on the laptop;
/// * still true: an alert that resolved is over, and a toast about it would be
///   the server repeating yesterday;
/// * never an agent's own silence. `no_agent_data` means nothing arrived from
///   any of the person's machines, so a machine able to ask has, by asking,
///   ended it - five minutes before the sweep notices.
///
/// `IS DISTINCT FROM` for the two comparisons against a column that is null on
/// most rows: a plain `<>` against null is null, and a null in a `WHERE` drops
/// the row without a word.
const PENDING_FOR_AGENT: &str = "
    FROM notifications n
    JOIN agents a ON a.id = $1
    JOIN users u ON u.id = a.user_id
    LEFT JOIN alerts al ON al.id = n.alert_id
    WHERE n.user_id = a.user_id
      AND n.id > greatest(a.notified_through, u.notifications_read_through)
      AND n.created_at >= a.created_at
      AND n.agent_id IS DISTINCT FROM a.id
      AND al.resolved_at IS NULL
      AND al.rule IS DISTINCT FROM 'no_agent_data'";

/// How many notices an agent has not shown. Answered on every pulse.
pub async fn pending_count(pool: &PgPool, agent_id: Uuid) -> Result<i64, ApiError> {
    // `AssertSqlSafe` on `PENDING_FOR_AGENT`, a constant in this module.
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) {PENDING_FOR_AGENT}")))
        .bind(agent_id)
        .fetch_one(pool)
        .await?)
}

/// How many notices one answer to an agent carries. More wait for the next
/// request; a machine back from a month offline is not handed them all at once.
const AGENT_PAGE: i64 = 20;

/// What an agent is told to show.
#[derive(Debug, Serialize)]
pub struct AgentQueue {
    /// Oldest first: toasts arrive in the order things happened.
    pub notifications: Vec<Notification>,
    /// Whether more are waiting past this page. An agent that has shown these
    /// and acknowledged them asks again.
    pub more: bool,
}

/// `GET /api/v1/agent/notifications`: what this machine has not shown yet.
pub async fn agent_queue(State(state): State<AppState>, agent: AuthenticatedAgent) -> Result<impl IntoResponse, ApiError> {
    // `AssertSqlSafe` on `PENDING_FOR_AGENT`, a constant in this module; the
    // agent and the limit are bound.
    let mut rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT n.id, n.kind, n.created_at, n.payload, al.resolved_at AS withdrawn_at, false AS read
         {PENDING_FOR_AGENT}
         ORDER BY n.id
         LIMIT $2"
    )))
    .bind(agent.agent_id)
    .bind(AGENT_PAGE + 1)
    .fetch_all(&state.pool)
    .await?;

    let more = rows.len() as i64 > AGENT_PAGE;
    rows.truncate(AGENT_PAGE as usize);

    Ok(Json(AgentQueue {
        notifications: rows.into_iter().map(|row| row.into_notification(&state.webhooks)).collect(),
        more,
    }))
}

/// How far a reader got.
#[derive(Debug, Deserialize)]
pub struct Through {
    /// The id of the last notice read or shown. Everything up to it counts.
    pub through: i64,
}

/// What a cursor stands at after moving.
#[derive(Debug, Serialize)]
pub struct Cursor {
    pub through: i64,
}

fn validate(through: &Through) -> Result<(), ApiError> {
    if through.through < 0 {
        return Err(ApiError::bad_request("`through` is the id of a notification, and ids start at 1"));
    }
    Ok(())
}

/// `POST /api/v1/agent/notifications/ack`: this machine has shown everything
/// up to `through`.
///
/// The cursor only moves forward - an acknowledgement that arrives late, after
/// a later one, must not bring back toasts already shown. It also never moves
/// past what exists for this person: an agent that sent a number from the
/// future, a bug or a careless constant, would otherwise silence every notice
/// not yet written, and nothing would ever say so.
pub async fn agent_ack(State(state): State<AppState>, agent: AuthenticatedAgent, Json(through): Json<Through>) -> Result<impl IntoResponse, ApiError> {
    validate(&through)?;
    let moved: i64 = sqlx::query_scalar(
        "UPDATE agents
         SET notified_through = greatest(notified_through,
                                         least($2, (SELECT coalesce(max(id), 0) FROM notifications WHERE user_id = agents.user_id)))
         WHERE id = $1
         RETURNING notified_through",
    )
    .bind(agent.agent_id)
    .bind(through.through)
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(Cursor { through: moved }))
}

/// `POST /api/v1/agent/notifications/read`: the person has seen everything up
/// to `through` - they clicked the toast, or read the list in kasl.
///
/// The same cursor the web inbox moves, because it is the same fact: the
/// person saw it, wherever they were. Other machines stop toasting it.
pub async fn agent_read(State(state): State<AppState>, agent: AuthenticatedAgent, Json(through): Json<Through>) -> Result<impl IntoResponse, ApiError> {
    validate(&through)?;
    let moved = mark_read(&state.pool, agent.user_id, through.through).await?;
    Ok(Json(Cursor { through: moved }))
}

/// Moves a person's read cursor forward, never past what they were sent.
async fn mark_read(pool: &PgPool, user_id: Uuid, through: i64) -> Result<i64, ApiError> {
    Ok(sqlx::query_scalar(
        "UPDATE users
         SET notifications_read_through = greatest(notifications_read_through,
                                                   least($2, (SELECT coalesce(max(id), 0) FROM notifications WHERE user_id = users.id)))
         WHERE id = $1
         RETURNING notifications_read_through",
    )
    .bind(user_id)
    .bind(through)
    .fetch_one(pool)
    .await?)
}

/// How much of the inbox one answer carries. The newest are what anybody
/// opens it for; this is a record of messages, not an archive to page through.
const INBOX_PAGE: i64 = 100;

/// A person's own inbox.
#[derive(Debug, Serialize)]
pub struct Inbox {
    /// Newest first.
    pub notifications: Vec<Notification>,
    /// Unread and still true: the badge. A notice that is over does not ask
    /// for attention, so it is not counted - it is still listed, as over.
    pub unread: i64,
    pub read_through: i64,
}

/// `GET /api/v1/me/notifications`: what the server has told you.
///
/// Yours alone. No role and no department in the query: what a person was told
/// is between the server and them, and the facts behind it are already visible
/// to the people entitled to them, on their own screens.
pub async fn inbox(State(state): State<AppState>, user: CurrentUser) -> Result<impl IntoResponse, ApiError> {
    let read_through: i64 = sqlx::query_scalar("SELECT notifications_read_through FROM users WHERE id = $1")
        .bind(user.user_id)
        .fetch_one(&state.pool)
        .await?;

    let rows: Vec<Row> = sqlx::query_as(
        "SELECT n.id, n.kind, n.created_at, n.payload, al.resolved_at AS withdrawn_at, n.id <= $2 AS read
         FROM notifications n
         LEFT JOIN alerts al ON al.id = n.alert_id
         WHERE n.user_id = $1
         ORDER BY n.id DESC
         LIMIT $3",
    )
    .bind(user.user_id)
    .bind(read_through)
    .bind(INBOX_PAGE)
    .fetch_all(&state.pool)
    .await?;

    let unread: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications n LEFT JOIN alerts al ON al.id = n.alert_id
         WHERE n.user_id = $1 AND n.id > $2 AND al.resolved_at IS NULL",
    )
    .bind(user.user_id)
    .bind(read_through)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(Inbox {
        notifications: rows.into_iter().map(|row| row.into_notification(&state.webhooks)).collect(),
        unread,
        read_through,
    }))
}

/// `POST /api/v1/me/notifications/read`: you have seen everything up to
/// `through`.
pub async fn read(State(state): State<AppState>, user: CurrentUser, Json(through): Json<Through>) -> Result<impl IntoResponse, ApiError> {
    validate(&through)?;
    let moved = mark_read(&state.pool, user.user_id, through.through).await?;
    Ok((StatusCode::OK, Json(Cursor { through: moved })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn alert(rule: AlertRule, observed: i64, against: Option<i64>, date: Option<&str>) -> Facts {
        Facts {
            alert: Some(AlertPayload {
                id: Uuid::nil(),
                rule,
                observed_seconds: observed,
                against_seconds: against,
                subject_date: date.map(|date| date.parse::<NaiveDate>().unwrap()),
                fired_at: "2026-09-22T10:00:00Z".parse().unwrap(),
            }),
            ..Facts::default()
        }
    }

    #[test]
    fn an_open_day_says_how_to_send_it_again() {
        // The one alert the employee can fix, and the notice says how: the
        // close kasl made never arrived, and this is the command that sends it.
        let (title, body) = words(
            1,
            NotificationKind::AlertRaised,
            &alert(AlertRule::DayNotClosed, 17 * 3600, Some(16 * 3600), Some("2026-09-22")),
        );
        assert_eq!(title, "Your day of 2026-09-22 is still open here");
        assert!(body.contains("17 h"), "{body}");
        assert!(body.contains("kasl server push --date 2026-09-22"), "{body}");
    }

    #[test]
    fn a_long_day_names_both_figures() {
        // What was measured and what against, so the reader can disagree with
        // the arithmetic - never a percentage (ADR 0017).
        let (title, body) = words(
            1,
            NotificationKind::AlertRaised,
            &alert(AlertRule::Overwork, 12 * 3600 + 36 * 60, Some(8 * 3600), Some("2026-09-17")),
        );
        assert_eq!(title, "Your manager was told about 2026-09-17");
        assert_eq!(body, "You worked 12.6 h that day, against your norm of 8 h.");
    }

    #[test]
    fn silence_is_told_in_its_own_unit() {
        let (_, body) = words(
            1,
            NotificationKind::AlertRaised,
            &alert(AlertRule::NoAgentData, 13 * 3600, Some(12 * 3600), None),
        );
        assert_eq!(body, "Nothing had arrived from any of your machines for 13 h.");
    }

    #[test]
    fn a_machine_is_named_as_it_was() {
        let facts = Facts {
            agent: Some(AgentFact {
                id: Uuid::nil(),
                name: "laptop".to_string(),
            }),
            ..Facts::default()
        };
        assert_eq!(
            words(1, NotificationKind::AgentIssued, &facts).0,
            "\u{201c}laptop\u{201d} can now report as you"
        );
        assert_eq!(
            words(1, NotificationKind::AgentRevoked, &facts).0,
            "\u{201c}laptop\u{201d} can no longer report as you"
        );
    }

    #[test]
    fn a_privacy_change_names_both_levels_and_what_is_kept_now() {
        let facts = Facts {
            privacy: Some(PrivacyFact {
                from: PrivacyLevel::Full,
                to: PrivacyLevel::Coarse,
            }),
            ..Facts::default()
        };
        let (_, body) = words(1, NotificationKind::PrivacyChanged, &facts);
        assert!(body.contains("from full to coarse"), "{body}");
        assert!(
            body.contains(crate::privacy::summary_for(PrivacyLevel::Coarse)),
            "the new level is described: {body}"
        );
    }

    #[test]
    fn a_notice_without_its_facts_still_says_something() {
        // Not an empty toast and not a panic: a row nothing here writes, which
        // a hand-edited backup could.
        let (title, body) = words(7, NotificationKind::AgentIssued, &Facts::default());
        assert!(!title.is_empty() && !body.is_empty());
    }

    #[test]
    fn the_wire_names_are_the_contract() {
        // kasl branches on these. A rename here is a new API version, not an
        // edit.
        for (kind, name) in [
            (NotificationKind::AlertRaised, "alert.raised"),
            (NotificationKind::AgentIssued, "agent.issued"),
            (NotificationKind::AgentRevoked, "agent.revoked"),
            (NotificationKind::PrivacyChanged, "privacy.changed"),
        ] {
            assert_eq!(serde_json::to_value(kind).unwrap(), serde_json::json!(name));
        }
    }

    #[test]
    fn only_the_facts_of_the_kind_travel() {
        // Flattened into the response: an alert notice carries `alert` and no
        // empty `agent` or `privacy` beside it.
        let json = serde_json::to_value(alert(AlertRule::Overwork, 1, None, None)).unwrap();
        let keys: Vec<&String> = json.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["alert"]);
    }
}
