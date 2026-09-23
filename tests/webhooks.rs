//! Webhooks, against a live database and a receiver on a local port.
//!
//! The receiver is a real HTTP server the dispatcher really posts to, so what
//! is checked here is what a channel would actually get: the path (and so the
//! credential in it), the headers, the body, the order. The dispatcher is
//! driven one tick at a time with a fixed `now`, the way the sweep is in the
//! alert tests - a test that waited for a thirty-second retry would be a test
//! nobody runs.

mod support;

use std::{
    collections::VecDeque,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
};
use chrono::{DateTime, Duration, Utc};
use kasl_server::webhooks::{self, Destination, Dispatched, Webhooks};
use serde_json::{Value, json};
use support::TestServer;

/// One request the receiver was sent.
#[derive(Debug, Clone)]
struct Seen {
    path: String,
    headers: HeaderMap,
    body: Bytes,
}

impl Seen {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("the body is JSON")
    }
}

#[derive(Clone, Default)]
struct Script {
    seen: Arc<Mutex<Vec<Seen>>>,
    /// What to answer, in order; `200 ok` once it runs out.
    answers: Arc<Mutex<VecDeque<(u16, &'static str)>>>,
}

/// A local HTTP server that records what it is sent.
struct Receiver {
    addr: SocketAddr,
    script: Script,
}

impl Receiver {
    async fn start(answers: &[(u16, &'static str)]) -> Self {
        let script = Script::default();
        script.answers.lock().unwrap().extend(answers.iter().copied());

        async fn take(State(script): State<Script>, uri: Uri, headers: HeaderMap, body: Bytes) -> (StatusCode, &'static str) {
            script.seen.lock().unwrap().push(Seen {
                path: uri.path().to_string(),
                headers,
                body,
            });
            let (status, text) = script.answers.lock().unwrap().pop_front().unwrap_or((200, "ok"));
            (StatusCode::from_u16(status).unwrap(), text)
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new().fallback(take).with_state(script.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { addr, script }
    }

    fn seen(&self) -> Vec<Seen> {
        self.script.seen.lock().unwrap().clone()
    }

    /// A hook whose path carries a credential, the way Slack's does.
    fn slack(&self, variable: &str, options: &str) -> Destination {
        Destination::parse(variable, &format!("slack http://{}/services/T0/B0/HOOKSECRET {options}", self.addr)).unwrap()
    }
}

/// Moves an agent's last contact back, which is how somebody becomes silent.
async fn last_seen(server: &TestServer, email: &str, at: DateTime<Utc>) {
    sqlx::query("UPDATE agents SET last_seen_at = $2 WHERE user_id = (SELECT id FROM users WHERE lower(email) = lower($1))")
        .bind(email)
        .bind(at)
        .execute(&server.pool)
        .await
        .unwrap();
}

async fn sweep(server: &TestServer, now: DateTime<Utc>) {
    kasl_server::alerts::sweep(&server.pool, now, server.webhooks())
        .await
        .expect("the sweep should run");
}

async fn dispatch(server: &TestServer, now: DateTime<Utc>) -> Dispatched {
    webhooks::dispatch_due(&server.pool, server.webhooks(), &webhooks::client(), now)
        .await
        .expect("the dispatch should run")
}

/// Every queued delivery as `(destination, event)`, oldest first.
async fn queued(server: &TestServer) -> Vec<(String, String)> {
    sqlx::query_as("SELECT destination, event::text FROM webhook_deliveries ORDER BY created_at, event::text")
        .fetch_all(&server.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_raised_alert_is_queued_with_it_and_reaches_the_channel() {
    let Some(server) = TestServer::start().await else { return };
    let receiver = Receiver::start(&[]).await;
    let server = server.with_webhooks(Webhooks::new(vec![receiver.slack("KASL_WEBHOOK_TEAM", "")], Some("https://kasl.example.com")));

    let now = Utc::now();
    last_seen(&server, "employee@example.test", now - Duration::hours(30)).await;
    sweep(&server, now).await;

    assert_eq!(queued(&server).await, vec![("team".to_string(), "alert.raised".to_string())]);

    let done = dispatch(&server, now).await;
    assert_eq!(done.delivered, 1, "{done:?}");

    let seen = receiver.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].path, "/services/T0/B0/HOOKSECRET", "posted to the hook itself");
    let text = seen[0].json()["text"].as_str().unwrap().to_string();
    assert!(text.starts_with("Needs attention: *employee*"), "{text}");
    assert!(text.contains("Nothing from their agent for 30 h"), "{text}");
    assert!(text.contains("<https://kasl.example.com/team/"), "the message links to the person: {text}");

    // Sent once: the next tick finds nothing, and a second sweep over the same
    // silence queues nothing new.
    assert_eq!(dispatch(&server, now + Duration::seconds(5)).await, Dispatched::default());
    sweep(&server, now + Duration::minutes(5)).await;
    assert_eq!(queued(&server).await.len(), 1);
    assert_eq!(receiver.seen().len(), 1);

    server.close().await;
}

#[tokio::test]
async fn a_destination_hears_only_what_it_subscribes_to_and_about_whom() {
    let Some(server) = TestServer::start().await else { return };
    let receiver = Receiver::start(&[]).await;
    let server = server.with_webhooks(Webhooks::new(
        vec![
            receiver.slack("KASL_WEBHOOK_EVERYONE", ""),
            receiver.slack("KASL_WEBHOOK_DAYS", "events=day.closed"),
            receiver.slack("KASL_WEBHOOK_DESIGN", "department=design"),
        ],
        None,
    ));

    // Nobody is in Design yet, so the design channel hears nothing - not even
    // about somebody with no department at all.
    let now = Utc::now();
    last_seen(&server, "employee@example.test", now - Duration::hours(30)).await;
    sweep(&server, now).await;
    assert_eq!(queued(&server).await, vec![("everyone".to_string(), "alert.raised".to_string())]);

    // Once they are, a new alert about them reaches it too - matched without
    // regard to case, as the department is typed by hand in the environment.
    sqlx::query("INSERT INTO departments (name) VALUES ('Design')")
        .execute(&server.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET department_id = (SELECT id FROM departments) WHERE email = 'employee@example.test'")
        .execute(&server.pool)
        .await
        .unwrap();
    let today = now.date_naive();
    let started = now - Duration::hours(20);
    let (status, _) = server
        .post_day(
            &server.token,
            json!({ "date": today.to_string(), "started_at": started.to_rfc3339(), "pauses": [], "tasks": [] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    sweep(&server, now).await;

    let rows = queued(&server).await;
    assert!(rows.contains(&("design".to_string(), "alert.raised".to_string())), "{rows:?}");
    assert!(!rows.iter().any(|(name, _)| name == "days"), "the days channel does not hear alerts: {rows:?}");

    server.close().await;
}

#[tokio::test]
async fn a_failing_receiver_is_retried_later_and_holds_its_later_events_back() {
    let Some(server) = TestServer::start().await else { return };
    let receiver = Receiver::start(&[(503, "down for maintenance")]).await;
    let server = server.with_webhooks(Webhooks::new(vec![receiver.slack("KASL_WEBHOOK_TEAM", "")], None));

    // Raised, then resolved before anything was delivered: two events queued
    // for one channel, in that order.
    let now = Utc::now();
    last_seen(&server, "employee@example.test", now - Duration::hours(30)).await;
    sweep(&server, now).await;
    last_seen(&server, "employee@example.test", now).await;
    sweep(&server, now + Duration::minutes(5)).await;
    assert_eq!(queued(&server).await.len(), 2);

    let first = now + Duration::minutes(5);
    let done = dispatch(&server, first).await;
    assert_eq!(
        done,
        Dispatched {
            delivered: 0,
            retrying: 1,
            abandoned: 0
        }
    );
    assert_eq!(receiver.seen().len(), 1, "the resolution waits behind the raise rather than overtaking it");

    let (attempts, error): (i32, String) = sqlx::query_as("SELECT attempts, last_error FROM webhook_deliveries WHERE event = 'alert.raised'")
        .fetch_one(&server.pool)
        .await
        .unwrap();
    assert_eq!(attempts, 1);
    assert!(error.contains("503") && error.contains("down for maintenance"), "{error}");

    // Not due yet: nothing is sent.
    assert_eq!(dispatch(&server, first + Duration::seconds(10)).await, Dispatched::default());

    // Due: both go, in the order they happened.
    let done = dispatch(&server, first + Duration::seconds(31)).await;
    assert_eq!(done.delivered, 2, "{done:?}");
    let texts: Vec<String> = receiver.seen().iter().map(|seen| seen.json()["text"].as_str().unwrap().to_string()).collect();
    assert!(texts[1].starts_with("Needs attention:"), "{texts:?}");
    assert!(texts[2].starts_with("Resolved:"), "{texts:?}");

    server.close().await;
}

#[tokio::test]
async fn a_refusal_is_final_and_no_error_keeps_the_address() {
    let Some(server) = TestServer::start().await else { return };
    let receiver = Receiver::start(&[(404, "no_service")]).await;
    let refused = receiver.slack("KASL_WEBHOOK_GONE", "");
    // Nothing listens on port 1: a connection error, which is retried.
    let unreachable = Destination::parse("KASL_WEBHOOK_DOWN", "slack http://127.0.0.1:1/services/T0/B0/HOOKSECRET").unwrap();
    let server = server.with_webhooks(Webhooks::new(vec![refused, unreachable], None));

    let now = Utc::now();
    last_seen(&server, "employee@example.test", now - Duration::hours(30)).await;
    sweep(&server, now).await;

    let done = dispatch(&server, now).await;
    assert_eq!(
        done,
        Dispatched {
            delivered: 0,
            retrying: 1,
            abandoned: 1
        }
    );

    let rows: Vec<(String, Option<i32>, String, bool)> =
        sqlx::query_as("SELECT destination, last_status, last_error, abandoned_at IS NOT NULL FROM webhook_deliveries ORDER BY destination")
            .fetch_all(&server.pool)
            .await
            .unwrap();

    let (_, _, error, abandoned) = &rows[0];
    assert_eq!(rows[0].0, "down");
    assert!(!abandoned, "a connection that failed is a bad moment, not a refusal");
    assert!(!error.is_empty());

    let (_, status, refusal, abandoned) = &rows[1];
    assert_eq!(rows[1].0, "gone");
    assert!(abandoned, "a hook that is gone stays gone");
    assert_eq!(*status, Some(404));
    assert!(refusal.contains("no_service"), "the receiver's reason is kept: {refusal}");

    // The address is the credential. A connection error from the HTTP client
    // names the URL it was sending to unless told not to - the failure this
    // guards is the delivery log becoming a list of working hooks.
    for (name, _, error, _) in &rows {
        assert!(!error.contains("HOOKSECRET"), "{name}: {error}");
    }

    server.close().await;
}

#[tokio::test]
async fn a_json_receiver_can_check_what_it_was_sent() {
    let Some(server) = TestServer::start().await else { return };
    let receiver = Receiver::start(&[]).await;
    let destination = Destination::parse("KASL_WEBHOOK_PAYROLL", &format!("json http://{}/in secret=shared-secret", receiver.addr)).unwrap();
    let server = server.with_webhooks(Webhooks::new(vec![destination], None));

    server.add_admin("root@example.test", "supersecret").await;
    let (_, cookie, _) = server.login("root@example.test", "supersecret").await;
    let (status, _, answer) = server.post_with_cookie("/api/v1/webhooks/payroll/test", cookie.as_deref(), json!({})).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{answer}");

    let now = Utc::now();
    assert_eq!(dispatch(&server, now).await.delivered, 1);

    let seen = &receiver.seen()[0];
    let body = seen.json();
    assert_eq!(body["event"], "test");
    assert_eq!(body["version"], 1);
    assert_eq!(body["id"], answer["event_id"]);
    assert_eq!(seen.headers["x-kasl-event"], "test");
    assert_eq!(seen.headers["x-kasl-event-id"].to_str().unwrap(), answer["event_id"].as_str().unwrap());

    // What a receiver does: take the timestamp from the header, sign the body
    // it got with the secret it holds, and compare.
    let header = seen.headers["x-kasl-signature"].to_str().unwrap().to_string();
    let timestamp: i64 = header.strip_prefix("t=").and_then(|rest| rest.split(',').next()).unwrap().parse().unwrap();
    assert_eq!(webhooks::signature("shared-secret", timestamp, &seen.body), header);
    assert_ne!(
        webhooks::signature("another-secret", timestamp, &seen.body),
        header,
        "a signature any secret produces checks nothing"
    );

    // And the test is in the audit log: posting into a team's channel is done
    // in the installation's name.
    let tested: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = 'webhook.tested'")
        .fetch_one(&server.pool)
        .await
        .unwrap();
    assert_eq!(tested, 1);

    server.close().await;
}

#[tokio::test]
async fn a_telegram_chat_is_posted_to_through_its_bot() {
    let Some(server) = TestServer::start().await else { return };
    let receiver = Receiver::start(&[]).await;
    let destination = Destination::parse("KASL_WEBHOOK_OPS", "telegram 123456:BOTSECRET chat=-100200300")
        .unwrap()
        .with_telegram_api(&format!("http://{}", receiver.addr));
    let server = server.with_webhooks(Webhooks::new(vec![destination], None));

    let now = Utc::now();
    last_seen(&server, "employee@example.test", now - Duration::hours(30)).await;
    sweep(&server, now).await;
    assert_eq!(dispatch(&server, now).await.delivered, 1);

    let seen = &receiver.seen()[0];
    assert_eq!(seen.path, "/bot123456:BOTSECRET/sendMessage");
    let body = seen.json();
    assert_eq!(body["chat_id"], "-100200300");
    assert_eq!(body["parse_mode"], "HTML");
    assert!(body["text"].as_str().unwrap().starts_with("Needs attention: <b>employee</b>"), "{body}");

    server.close().await;
}

#[tokio::test]
async fn acknowledging_tells_the_channel_who_looked() {
    let Some(server) = TestServer::start().await else { return };
    let receiver = Receiver::start(&[]).await;
    let server = server.with_webhooks(Webhooks::new(vec![receiver.slack("KASL_WEBHOOK_TEAM", "")], None));

    let now = Utc::now();
    server.add_admin("boss@example.test", "supersecret").await;
    last_seen(&server, "employee@example.test", now - Duration::hours(30)).await;
    sweep(&server, now).await;

    let (_, cookie, _) = server.login("boss@example.test", "supersecret").await;
    let (_, feed) = server.get_with_cookie("/api/v1/alerts", cookie.as_deref()).await;
    let id = feed["alerts"][0]["id"].as_str().unwrap().to_string();
    let (status, _, _) = server
        .post_with_cookie(&format!("/api/v1/alerts/{id}/acknowledge"), cookie.as_deref(), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);

    let by: String = sqlx::query_scalar("SELECT payload->>'by' FROM webhook_deliveries WHERE event = 'alert.acknowledged'")
        .fetch_one(&server.pool)
        .await
        .expect("the acknowledgement is queued with the alert's answer");
    assert_eq!(by, "boss@example.test");

    // The condition ending later is still news for the channel, answered or
    // not: it was told the alert was raised and that somebody looked.
    last_seen(&server, "employee@example.test", now).await;
    sweep(&server, now + Duration::minutes(5)).await;
    let events: Vec<String> = queued(&server).await.into_iter().map(|(_, event)| event).collect();
    assert_eq!(events, ["alert.raised", "alert.acknowledged", "alert.resolved"]);

    server.close().await;
}

#[tokio::test]
async fn a_day_is_announced_once_when_it_first_arrives_finished() {
    let Some(server) = TestServer::start().await else { return };
    let receiver = Receiver::start(&[]).await;
    let server = server.with_webhooks(Webhooks::new(vec![receiver.slack("KASL_WEBHOOK_DAYS", "events=day.closed")], None));

    let now = Utc::now();
    let started = now - Duration::hours(9);
    let date = started.date_naive().to_string();
    let open = json!({ "date": date, "started_at": started.to_rfc3339(), "pauses": [], "tasks": [] });
    let closed = json!({
        "date": date,
        "started_at": started.to_rfc3339(),
        "ended_at": (now - Duration::hours(1)).to_rfc3339(),
        "pauses": [{ "started_at": (started + Duration::hours(4)).to_rfc3339(), "ended_at": (started + Duration::hours(5)).to_rfc3339(), "duration_seconds": 3600 }],
        "tasks": []
    });

    server.post_day(&server.token, open.clone()).await;
    assert!(queued(&server).await.is_empty(), "an open day is not news");

    server.post_day(&server.token, closed.clone()).await;
    assert_eq!(queued(&server).await, vec![("days".to_string(), "day.closed".to_string())]);
    let worked: i64 = sqlx::query_scalar("SELECT (payload->'day'->>'worked_seconds')::bigint FROM webhook_deliveries")
        .fetch_one(&server.pool)
        .await
        .unwrap();
    assert_eq!(worked, 7 * 3600, "eight hours of span less an hour's pause");

    // The agent re-sends the finished day - a corrected task, a retry after
    // a lost answer. Still one announcement.
    server.post_day(&server.token, closed.clone()).await;
    assert_eq!(queued(&server).await.len(), 1);

    // Or corrects when it ended. A different end makes a different event id,
    // so this is the case the id alone would announce twice: the day was
    // already closed, and a corrected close is not a second one.
    let mut corrected = closed;
    corrected["ended_at"] = json!((now - Duration::minutes(30)).to_rfc3339());
    server.post_day(&server.token, corrected).await;
    assert_eq!(queued(&server).await.len(), 1, "a corrected end is not a new close");

    // A fortnight of backfill arrives after time offline. Those days were
    // finished long ago; announcing each now would be noise, not news.
    let old = now - Duration::days(10);
    let (status, _) = server
        .post_batch(
            &server.token,
            json!([{ "date": old.date_naive().to_string(), "started_at": old.to_rfc3339(), "ended_at": (old + Duration::hours(8)).to_rfc3339(), "pauses": [], "tasks": [] }]),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(queued(&server).await.len(), 1);

    server.close().await;
}

#[tokio::test]
async fn a_destination_removed_from_the_environment_is_given_up_on_in_writing() {
    let Some(server) = TestServer::start().await else { return };
    let receiver = Receiver::start(&[]).await;
    let server = server.with_webhooks(Webhooks::new(vec![receiver.slack("KASL_WEBHOOK_TEAM", "")], None));

    let now = Utc::now();
    last_seen(&server, "employee@example.test", now - Duration::hours(30)).await;
    sweep(&server, now).await;

    // The operator removed the variable and restarted before it went out.
    let server = server.with_webhooks(Webhooks::default());
    let done = dispatch(&server, now).await;
    assert_eq!(done.abandoned, 1);
    let error: String = sqlx::query_scalar("SELECT last_error FROM webhook_deliveries")
        .fetch_one(&server.pool)
        .await
        .unwrap();
    assert!(error.contains("`team`"), "{error}");
    assert!(receiver.seen().is_empty());

    server.close().await;
}

#[tokio::test]
async fn the_overview_is_for_administrators_and_names_no_address() {
    let Some(server) = TestServer::start().await else { return };
    let receiver = Receiver::start(&[]).await;
    let server = server.with_webhooks(Webhooks::new(
        vec![
            receiver.slack("KASL_WEBHOOK_TEAM", ""),
            receiver.slack("KASL_WEBHOOK_SALES", "department=Sales"),
        ],
        None,
    ));

    server.add_admin("root@example.test", "supersecret").await;
    let (_, cookie, _) = server.login("root@example.test", "supersecret").await;
    let (status, overview) = server.get_with_cookie("/api/v1/webhooks", cookie.as_deref()).await;
    assert_eq!(status, StatusCode::OK);

    assert!(!overview.to_string().contains("HOOKSECRET"), "the screen never carries a hook: {overview}");
    let sales = &overview["destinations"][0];
    assert_eq!(sales["name"], "sales");
    assert_eq!(sales["target"], "127.0.0.1");
    // A department nobody has: a destination that will hear nothing, and the
    // one place that can say so.
    assert_eq!(sales["department_exists"], false);
    assert_eq!(overview["destinations"][1]["department_exists"], Value::Null);
    assert_eq!(overview["links"], false);

    // The manifest names both, by label - an employee reads there what leaves
    // the server about them.
    let (_, manifest) = server.get_with_cookie("/api/v1/privacy", cookie.as_deref()).await;
    let sent = manifest["sent_elsewhere"].as_array().expect("the manifest lists what leaves the server");
    assert_eq!(sent.len(), 2);
    assert!(!manifest.to_string().contains("HOOKSECRET"));

    // An employee may read the manifest and not the screen.
    server.set_password("employee@example.test", "employeepass").await;
    let (_, employee, _) = server.login("employee@example.test", "employeepass").await;
    let (status, _) = server.get_with_cookie("/api/v1/webhooks", employee.as_deref()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) = server.post_with_cookie("/api/v1/webhooks/team/test", employee.as_deref(), json!({})).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _, _) = server.post_with_cookie("/api/v1/webhooks/nobody/test", cookie.as_deref(), json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    server.close().await;
}
