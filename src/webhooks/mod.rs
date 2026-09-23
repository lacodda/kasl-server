//! Webhooks: the server says what it noticed to where a manager already is.
//!
//! Alerts made the server notice things on its own (ADR 0018), and then wait
//! on a dashboard for somebody to open it. This module takes the same record
//! outward - to a Slack or Mattermost channel, a Telegram chat, or a system of
//! the operator's own - without replacing it: delivery is added to the record,
//! and the row an alert wrote is what gets sent.
//!
//! Three decisions shape everything here (ADR 0019):
//!
//! * **Destinations live in the environment, not in the database.** A hook URL
//!   and a bot token are working credentials to post as somebody, and the
//!   database is what `kasl-server backup` writes to a file. They sit next to
//!   the database password, in `KASL_WEBHOOK_<NAME>`, and everything else -
//!   the delivery log, the screen, the privacy manifest - knows a destination
//!   by its name alone.
//! * **An event is queued in the transaction that made it true.** The alert
//!   row and the delivery row commit together, or neither does. An in-memory
//!   channel loses what was queued on every restart, and "the server restarted
//!   at the moment the agent died" is exactly the night nobody hears about.
//! * **Delivery is at least once, in order, per destination.** The dispatcher
//!   retries with growing gaps and gives up in writing after about a day; a
//!   destination that is failing holds its own later events back rather than
//!   letting "resolved" overtake "raised" in somebody's channel. Every event
//!   carries an id a receiver can deduplicate on, because a request that timed
//!   out may still have arrived.

mod destination;
mod dispatch;
mod render;
mod sign;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgConnection;
use uuid::Uuid;

pub use destination::{Destination, EventKind, Kind, PREFIX};
pub use dispatch::{Dispatched, client, dispatch_due, run_dispatcher};
pub use sign::{hmac_sha256, signature};

use crate::{alerts::AlertRule, app::AppState, audit, calendar::WorkdayKind, error::ApiError, login::CurrentUser};

/// The payload's shape. A receiver of the `json` kind reads this first; a
/// change to what a field means is a new number, not a quiet edit.
pub const PAYLOAD_VERSION: u32 = 1;

/// Every destination this installation sends to, and where its screens live.
#[derive(Debug, Clone, Default)]
pub struct Webhooks {
    /// Sorted by name, so the screen and the manifest list them the same way
    /// on every start.
    destinations: Vec<Destination>,
    /// The address people open the web UI at (`KASL_PUBLIC_URL`), without a
    /// trailing slash. When set, a message links to the person it is about;
    /// when not, it says what happened and leaves the finding to the reader.
    public_url: Option<String>,
}

impl Webhooks {
    /// Reads every `KASL_WEBHOOK_*` variable among `vars`.
    ///
    /// An error in any one stops the server from starting, and names the
    /// variable without repeating its value.
    pub fn from_vars(vars: impl IntoIterator<Item = (String, String)>, public_url: Option<String>) -> Result<Self, String> {
        let mut destinations = vars
            .into_iter()
            .filter(|(key, _)| key.starts_with(PREFIX))
            .map(|(key, value)| Destination::parse(&key, &value))
            .collect::<Result<Vec<_>, _>>()?;
        destinations.sort_by(|a, b| a.name.cmp(&b.name));

        let public_url = match public_url.map(|url| url.trim().trim_end_matches('/').to_string()) {
            Some(url) if url.is_empty() => None,
            Some(url) if url.starts_with("https://") || url.starts_with("http://") => Some(url),
            Some(_) => return Err("KASL_PUBLIC_URL is not an http(s) address".to_string()),
            None => None,
        };

        Ok(Self { destinations, public_url })
    }

    /// A set built in code, for the tests.
    pub fn new(destinations: Vec<Destination>, public_url: Option<&str>) -> Self {
        let mut destinations = destinations;
        destinations.sort_by(|a, b| a.name.cmp(&b.name));
        Self {
            destinations,
            public_url: public_url.map(|url| url.trim_end_matches('/').to_string()),
        }
    }

    pub fn destinations(&self) -> &[Destination] {
        &self.destinations
    }

    pub fn get(&self, name: &str) -> Option<&Destination> {
        self.destinations.iter().find(|destination| destination.name == name)
    }

    /// Whether any destination hears `event` - checked before assembling one,
    /// so an installation with no webhooks pays nothing for them.
    pub fn anyone_hears(&self, event: EventKind) -> bool {
        self.destinations.iter().any(|destination| destination.hears(event))
    }

    fn link(&self, path: &str) -> Option<String> {
        self.public_url.as_ref().map(|base| format!("{base}{path}"))
    }
}

// The event ---------------------------------------------------------------------

/// One thing that happened, as every destination is told it.
///
/// This is the `json` kind's body verbatim, and what the chat kinds render
/// their text from - so a Slack message can never say something the payload
/// does not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub version: u32,
    /// The same for every destination this event went to, and for every retry
    /// to one. What a receiver deduplicates on.
    pub id: Uuid,
    pub event: EventKind,
    pub occurred_at: DateTime<Utc>,
    /// The server that said it, so a receiver fed by several can tell them
    /// apart and a message can name the version that sent it.
    pub server_version: String,
    /// Where to look, when the installation knows its own address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// Who it is about. Absent from a test.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub person: Option<Person>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alert: Option<AlertPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day: Option<DayPayload>,
    /// Who answered an alert, by name - on `alert.acknowledged` only. So the
    /// channel knows it has been looked at and two managers do not both
    /// chase it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
}

/// The person an event is about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Person {
    pub id: Uuid,
    pub name: String,
    pub department: Option<String>,
}

/// An alert as it fired. The figures are the ones it fired on, never
/// re-derived: a message already read must not change under its reader
/// (ADR 0018).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
pub struct AlertPayload {
    pub id: Uuid,
    pub rule: AlertRule,
    pub observed_seconds: i64,
    pub against_seconds: Option<i64>,
    pub subject_date: Option<NaiveDate>,
    pub fired_at: DateTime<Utc>,
}

/// A day as it arrived finished.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DayPayload {
    pub date: NaiveDate,
    pub kind: WorkdayKind,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    /// The span minus the pauses, in seconds.
    pub worked_seconds: i64,
}

impl Event {
    fn new(event: EventKind, id: Uuid, now: DateTime<Utc>) -> Self {
        Self {
            version: PAYLOAD_VERSION,
            id,
            event,
            occurred_at: now,
            server_version: env!("CARGO_PKG_VERSION").to_string(),
            link: None,
            person: None,
            alert: None,
            day: None,
            by: None,
        }
    }

    /// A step in an alert's life.
    pub fn alert(event: EventKind, alert: AlertPayload, person: Person, by: Option<String>, webhooks: &Webhooks, now: DateTime<Utc>) -> Self {
        Self {
            // One id per alert per step: the sweep that raises it and a
            // second sweep racing it produce the same event, which the unique
            // key then refuses to queue twice.
            link: webhooks.link(&format!("/team/{}", person.id)),
            person: Some(person),
            by,
            ..Self::new(event, derived_id(event, &[alert.id.as_bytes()]), now)
        }
        .with_alert(alert)
    }

    fn with_alert(mut self, alert: AlertPayload) -> Self {
        self.alert = Some(alert);
        self
    }

    /// A day that has just arrived finished.
    pub fn day_closed(workday_id: Uuid, day: DayPayload, person: Person, webhooks: &Webhooks, now: DateTime<Utc>) -> Self {
        // Keyed on the day and its end, so the same close re-sent is the same
        // event, and a day reopened and closed again later is a new one.
        let id = derived_id(EventKind::DayClosed, &[workday_id.as_bytes(), day.ended_at.to_rfc3339().as_bytes()]);
        Self {
            link: webhooks.link(&format!("/team/{}", person.id)),
            person: Some(person),
            day: Some(day),
            ..Self::new(EventKind::DayClosed, id, now)
        }
    }

    /// What an administrator sends to see a channel work.
    pub fn test(webhooks: &Webhooks, now: DateTime<Utc>) -> Self {
        Self {
            link: webhooks.link("/"),
            ..Self::new(EventKind::Test, Uuid::new_v4(), now)
        }
    }
}

/// An id that is a function of what the event is about.
///
/// Deterministic rather than random, which is what makes queuing idempotent:
/// `(event_id, destination)` is unique, so the same fact reached twice - two
/// overlapping sweeps, a retried upload - is queued once.
fn derived_id(event: EventKind, parts: &[&[u8]]) -> Uuid {
    let mut hash = Sha256::new();
    hash.update(event.name().as_bytes());
    for part in parts {
        hash.update([0u8]);
        hash.update(part);
    }
    let digest = hash.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    uuid::Builder::from_custom_bytes(bytes).into_uuid()
}

/// Reads who an event is about.
pub async fn person(conn: &mut PgConnection, user_id: Uuid) -> Result<Person, ApiError> {
    Ok(sqlx::query_as(
        "SELECT u.id, u.display_name AS name, d.name AS department
         FROM users u LEFT JOIN departments d ON d.id = u.department_id
         WHERE u.id = $1",
    )
    .bind(user_id)
    .fetch_one(conn)
    .await?)
}

/// Queues an event for every destination that hears it and may hear about
/// this person. Returns how many rows were written.
///
/// Takes the caller's connection so it runs inside the caller's transaction:
/// the fact and its announcement commit together or not at all.
pub async fn enqueue(conn: &mut PgConnection, webhooks: &Webhooks, event: &Event) -> Result<u64, ApiError> {
    let department = event.person.as_ref().and_then(|person| person.department.as_deref());
    let mut queued = 0;
    for destination in &webhooks.destinations {
        if !destination.hears(event.event) {
            continue;
        }
        // A destination for one department hears about nobody else - not
        // about people with no department either. The same boundary every
        // screen draws around a manager (ADR 0009).
        if let Some(wanted) = &destination.department
            && !department.is_some_and(|actual| actual.eq_ignore_ascii_case(wanted))
        {
            continue;
        }
        queued += enqueue_to(conn, destination, event).await?;
    }
    Ok(queued)
}

/// Queues an event for one destination, whatever it subscribes to.
async fn enqueue_to(conn: &mut PgConnection, destination: &Destination, event: &Event) -> Result<u64, ApiError> {
    let written = sqlx::query(
        "INSERT INTO webhook_deliveries (event_id, destination, event, user_id, payload, created_at, next_attempt_at)
         VALUES ($1, $2, $3, $4, $5, $6, $6)
         ON CONFLICT (event_id, destination) DO NOTHING",
    )
    .bind(event.id)
    .bind(&destination.name)
    .bind(event.event)
    .bind(event.person.as_ref().map(|person| person.id))
    .bind(sqlx::types::Json(event))
    .bind(event.occurred_at)
    .execute(conn)
    .await?;
    Ok(written.rows_affected())
}

// The API -------------------------------------------------------------------------

/// One destination, as the screen shows it.
#[derive(Debug, Serialize)]
pub struct DestinationView {
    pub name: String,
    pub kind: Kind,
    /// The host or the chat - never the credential.
    pub target: String,
    pub events: Vec<EventKind>,
    pub department: Option<String>,
    /// Whether the department it names exists. `false` is a destination that
    /// will hear nothing at all - a rename, or a typo in the environment -
    /// and the one place that can say so is here.
    pub department_exists: Option<bool>,
    pub pending: i64,
    pub delivered: i64,
    pub abandoned: i64,
    pub last_delivered_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub last_error_at: Option<DateTime<Utc>>,
}

/// One delivery, newest first on the screen.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct DeliveryView {
    pub id: Uuid,
    pub event_id: Uuid,
    pub destination: String,
    pub event: EventKind,
    pub person: Option<String>,
    pub created_at: DateTime<Utc>,
    pub attempts: i32,
    pub next_attempt_at: DateTime<Utc>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub abandoned_at: Option<DateTime<Utc>>,
    pub last_status: Option<i32>,
    pub last_error: Option<String>,
}

/// The screen: where events go, and how the last ones went.
#[derive(Debug, Serialize)]
pub struct Overview {
    pub destinations: Vec<DestinationView>,
    pub recent: Vec<DeliveryView>,
    /// Whether messages can link back. Said so an operator who sees plain
    /// text in the channel knows which setting would add the link.
    pub links: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct Counts {
    destination: String,
    pending: i64,
    delivered: i64,
    abandoned: i64,
    last_delivered_at: Option<DateTime<Utc>>,
}

/// Answers the destinations and the recent deliveries. Administrators only:
/// the log names people and the screen names where their alerts go.
pub async fn overview(State(state): State<AppState>, user: CurrentUser) -> Result<impl IntoResponse, ApiError> {
    user.require_admin()?;

    let counts: Vec<Counts> = sqlx::query_as(
        "SELECT destination,
                count(*) FILTER (WHERE delivered_at IS NULL AND abandoned_at IS NULL) AS pending,
                count(*) FILTER (WHERE delivered_at IS NOT NULL) AS delivered,
                count(*) FILTER (WHERE abandoned_at IS NOT NULL) AS abandoned,
                max(delivered_at) AS last_delivered_at
         FROM webhook_deliveries GROUP BY destination",
    )
    .fetch_all(&state.pool)
    .await?;

    let failures: Vec<(String, String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT DISTINCT ON (destination) destination, last_error, coalesce(abandoned_at, next_attempt_at)
         FROM webhook_deliveries
         WHERE last_error IS NOT NULL AND delivered_at IS NULL
         ORDER BY destination, created_at DESC",
    )
    .fetch_all(&state.pool)
    .await?;

    let departments: Vec<String> = sqlx::query_scalar("SELECT name FROM departments").fetch_all(&state.pool).await?;

    let destinations = state
        .webhooks
        .destinations
        .iter()
        .map(|destination| {
            let count = counts.iter().find(|count| count.destination == destination.name);
            let failure = failures.iter().find(|(name, ..)| *name == destination.name);
            DestinationView {
                name: destination.name.clone(),
                kind: destination.kind,
                target: destination.shown_target(),
                events: destination.events.clone(),
                department: destination.department.clone(),
                department_exists: destination
                    .department
                    .as_ref()
                    .map(|wanted| departments.iter().any(|name| name.eq_ignore_ascii_case(wanted))),
                pending: count.map_or(0, |count| count.pending),
                delivered: count.map_or(0, |count| count.delivered),
                abandoned: count.map_or(0, |count| count.abandoned),
                last_delivered_at: count.and_then(|count| count.last_delivered_at),
                last_error: failure.map(|(_, error, _)| error.clone()),
                last_error_at: failure.map(|(.., at)| *at),
            }
        })
        .collect();

    let recent: Vec<DeliveryView> = sqlx::query_as(
        "SELECT w.id, w.event_id, w.destination, w.event, u.display_name AS person, w.created_at, w.attempts,
                w.next_attempt_at, w.delivered_at, w.abandoned_at, w.last_status, w.last_error
         FROM webhook_deliveries w LEFT JOIN users u ON u.id = w.user_id
         ORDER BY w.created_at DESC, w.id
         LIMIT 50",
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(Overview {
        destinations,
        recent,
        links: state.webhooks.public_url.is_some(),
    }))
}

/// Queues a test message for one destination. Administrators only.
///
/// Queued like any event rather than sent inline, so the test exercises the
/// path real alerts take - the dispatcher, its retries, its log - and the
/// screen shows the outcome in the same list.
pub async fn send_test(State(state): State<AppState>, user: CurrentUser, Path(name): Path<String>) -> Result<impl IntoResponse, ApiError> {
    user.require_admin()?;

    let Some(destination) = state.webhooks.get(&name) else {
        return Err(ApiError::new(StatusCode::NOT_FOUND, format!("no destination named `{name}` is configured")));
    };

    let event = Event::test(&state.webhooks, Utc::now());
    let mut conn = state.pool.acquire().await?;
    enqueue_to(&mut conn, destination, &event).await?;
    drop(conn);

    // Posting into a team's channel is something done in the installation's
    // name, and the log is where "who sent that?" gets answered.
    audit::Entry::new(audit::action::WEBHOOK_TESTED)
        .by(user.user_id)
        .by_email(&user.email)
        .with(serde_json::json!({ "destination": destination.name, "event_id": event.id }))
        .record(&state.pool)
        .await;

    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "event_id": event.id, "destination": destination.name })),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slack(name: &str, options: &str) -> Destination {
        Destination::parse(name, &format!("slack https://hooks.slack.com/services/T/B/X {options}")).unwrap()
    }

    #[test]
    fn every_webhook_variable_is_read_and_nothing_else() {
        let webhooks = Webhooks::from_vars(
            [
                ("KASL_WEBHOOK_ZED".to_string(), "slack https://hooks.slack.com/z".to_string()),
                ("KASL_WEBHOOK_ALPHA".to_string(), "slack https://hooks.slack.com/a".to_string()),
                ("KASL_AGENTS".to_string(), "a@b.c:token".to_string()),
                ("PATH".to_string(), "/usr/bin".to_string()),
            ],
            Some("https://kasl.example.com/".to_string()),
        )
        .unwrap();
        let names: Vec<&str> = webhooks.destinations().iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["alpha", "zed"], "sorted, and only the webhook variables");
        assert_eq!(webhooks.link("/team/1").as_deref(), Some("https://kasl.example.com/team/1"));
    }

    #[test]
    fn one_bad_destination_stops_the_start() {
        let error = Webhooks::from_vars(
            [
                ("KASL_WEBHOOK_GOOD".to_string(), "slack https://hooks.slack.com/a".to_string()),
                ("KASL_WEBHOOK_BAD".to_string(), "slak https://hooks.slack.com/b".to_string()),
            ],
            None,
        )
        .unwrap_err();
        assert!(error.contains("KASL_WEBHOOK_BAD"), "{error}");

        let error = Webhooks::from_vars([], Some("kasl.example.com".to_string())).unwrap_err();
        assert!(error.contains("KASL_PUBLIC_URL"), "{error}");
    }

    #[test]
    fn the_same_fact_gets_the_same_id() {
        let alert = Uuid::new_v4();
        assert_eq!(
            derived_id(EventKind::AlertRaised, &[alert.as_bytes()]),
            derived_id(EventKind::AlertRaised, &[alert.as_bytes()])
        );
        assert_ne!(
            derived_id(EventKind::AlertRaised, &[alert.as_bytes()]),
            derived_id(EventKind::AlertResolved, &[alert.as_bytes()]),
            "raising and resolving one alert are two events",
        );
    }

    #[test]
    fn anyone_hears_follows_the_subscriptions() {
        let webhooks = Webhooks::new(vec![slack("KASL_WEBHOOK_A", "events=day.closed")], None);
        assert!(webhooks.anyone_hears(EventKind::DayClosed));
        assert!(!webhooks.anyone_hears(EventKind::AlertRaised));
        assert!(!Webhooks::default().anyone_hears(EventKind::AlertRaised));
    }
}
