//! Notifications to the employee, against a live database.
//!
//! The wording is unit-tested in the module. What is only reachable here is
//! the delivery: that a notice is written with the fact that made it true,
//! that an agent is told what it has not shown - and nothing it should not be
//! told - and that the two cursors, per machine and per person, only ever move
//! forward and never past what exists (ADR 0020).

mod support;

use axum::http::StatusCode;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde_json::{Value, json};
use support::TestServer;
use uuid::Uuid;

const EMPLOYEE: &str = "employee@example.test";

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

/// Pulses as `token` and answers how many notices the server says are waiting.
async fn pulse(server: &TestServer, token: &str) -> i64 {
    let (status, body) = server
        .post_with_header(
            "/api/v1/agent/heartbeat",
            Some(&bearer(token)),
            json!({ "state": "working", "at": Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true) }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    body["notifications"].as_i64().unwrap_or_else(|| panic!("the pulse answers a count: {body}"))
}

/// What `token`'s machine is told to show.
async fn queue(server: &TestServer, token: &str) -> Value {
    let (status, body) = server.get_with_header("/api/v1/agent/notifications", Some(&bearer(token))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

fn kinds(queue: &Value) -> Vec<String> {
    queue["notifications"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|n| n["kind"].as_str().unwrap().to_string())
        .collect()
}

async fn ack(server: &TestServer, token: &str, through: i64) -> (StatusCode, Value) {
    server
        .post_with_header("/api/v1/agent/notifications/ack", Some(&bearer(token)), json!({ "through": through }))
        .await
}

async fn user_id(server: &TestServer, email: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM users WHERE lower(email) = lower($1)")
        .bind(email)
        .fetch_one(&server.pool)
        .await
        .expect("the fixture account should exist")
}

async fn sweep(server: &TestServer, now: DateTime<Utc>) -> kasl_server::alerts::Swept {
    kasl_server::alerts::sweep(&server.pool, now, &kasl_server::webhooks::Webhooks::default())
        .await
        .expect("the sweep should run")
}

/// Leaves a day open since `hours` ago, and makes the agent look alive - so
/// the sweep raises `day_not_closed` and nothing else.
async fn leave_a_day_open(server: &TestServer, token: &str, now: DateTime<Utc>, hours: i64) -> String {
    let started = now - Duration::hours(hours);
    let date = started.date_naive().to_string();
    let (status, body) = server
        .post_day(token, json!({ "date": date, "started_at": started.to_rfc3339(), "pauses": [], "tasks": [] }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    date
}

/// Signs the employee in to the web UI.
async fn employee_session(server: &TestServer) -> Option<String> {
    server.set_password(EMPLOYEE, "correct horse").await;
    let (status, cookie, body) = server.login(EMPLOYEE, "correct horse").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    cookie
}

#[tokio::test]
async fn an_open_day_reaches_the_agent_on_its_next_pulse() {
    let Some(server) = TestServer::start().await else { return };
    let now = Utc::now();

    // Nothing yet. The seeded token's own "can now report as you" is not told
    // to the machine it is about.
    assert_eq!(pulse(&server, &server.token).await, 0);

    let date = leave_a_day_open(&server, &server.token, now, 20).await;
    assert_eq!(sweep(&server, now).await.raised, 1);

    assert_eq!(pulse(&server, &server.token).await, 1, "the pulse says one is waiting");
    let body = queue(&server, &server.token).await;
    assert_eq!(kinds(&body), ["alert.raised"]);
    let notice = &body["notifications"][0];
    assert_eq!(notice["alert"]["rule"], "day_not_closed");
    assert_eq!(notice["alert"]["subject_date"], date, "the agent can act on the date: {notice}");
    assert_eq!(notice["title"], format!("Your day of {date} is still open here"));
    assert!(
        notice["body"].as_str().unwrap().contains(&format!("kasl server push --date {date}")),
        "{notice}"
    );
    assert_eq!(notice["read"], false);
    assert!(notice["withdrawn_at"].is_null());
    assert_eq!(body["more"], false);

    // Shown, and said so: the pulse goes quiet and the queue is empty.
    let (status, cursor) = ack(&server, &server.token, notice["id"].as_i64().unwrap()).await;
    assert_eq!(status, StatusCode::OK, "{cursor}");
    assert_eq!(cursor["through"], notice["id"]);
    assert_eq!(pulse(&server, &server.token).await, 0);
    assert!(kinds(&queue(&server, &server.token).await).is_empty());

    // Still unread in the inbox: shown on a machine is not seen by a person.
    // Two - the alert, and the seeded token's own notice, which the desktop
    // was never shown and the person never read.
    let cookie = employee_session(&server).await;
    let (_, inbox) = server.get_with_cookie("/api/v1/me/notifications", cookie.as_deref()).await;
    assert_eq!(inbox["unread"], 2, "{inbox}");
    assert_eq!(inbox["notifications"][0]["kind"], "alert.raised", "newest first: {inbox}");
    assert_eq!(inbox["notifications"][0]["read"], false);

    server.close().await;
}

#[tokio::test]
async fn one_alert_is_told_once() {
    let Some(server) = TestServer::start().await else { return };
    let now = Utc::now();
    leave_a_day_open(&server, &server.token, now, 20).await;

    // Two sweeps: the second finds the alert open and raises nothing, so it
    // writes nothing either. And the table refuses a second notice for one
    // alert even when something does try, which is the race two overlapping
    // sweeps would run.
    sweep(&server, now).await;
    sweep(&server, now + Duration::minutes(5)).await;
    let alert: kasl_server::webhooks::AlertPayload = sqlx::query_as("SELECT id, rule, observed_seconds, against_seconds, subject_date, fired_at FROM alerts")
        .fetch_one(&server.pool)
        .await
        .unwrap();
    let mut conn = server.pool.acquire().await.unwrap();
    kasl_server::notifications::alert_raised(&mut conn, user_id(&server, EMPLOYEE).await, &alert)
        .await
        .unwrap();
    drop(conn);

    let told: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind = 'alert.raised'")
        .fetch_one(&server.pool)
        .await
        .unwrap();
    assert_eq!(told, 1);

    server.close().await;
}

#[tokio::test]
async fn a_notice_that_is_over_is_not_toasted_and_stays_in_the_inbox() {
    let Some(server) = TestServer::start().await else { return };
    let now = Utc::now();
    let date = leave_a_day_open(&server, &server.token, now, 20).await;
    sweep(&server, now).await;

    // The close finally arrives, and the next sweep resolves the alert.
    let started = now - Duration::hours(20);
    let (status, body) = server
        .post_day(
            &server.token,
            json!({ "date": date, "started_at": started.to_rfc3339(), "ended_at": (started + Duration::hours(8)).to_rfc3339(), "pauses": [], "tasks": [] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(sweep(&server, now + Duration::minutes(5)).await.resolved, 1);

    assert_eq!(pulse(&server, &server.token).await, 0, "a resolved alert is not news");
    assert!(kinds(&queue(&server, &server.token).await).is_empty());

    let cookie = employee_session(&server).await;
    let (_, inbox) = server.get_with_cookie("/api/v1/me/notifications", cookie.as_deref()).await;
    let alert = inbox["notifications"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["kind"] == "alert.raised")
        .expect("kept in the inbox");
    assert!(alert["withdrawn_at"].is_string(), "shown as over: {alert}");
    assert_eq!(inbox["unread"], 1, "what is over does not ask for attention; the seeded token does: {inbox}");

    server.close().await;
}

#[tokio::test]
async fn an_agent_is_never_told_about_its_own_silence() {
    let Some(server) = TestServer::start().await else { return };
    let now = Utc::now();

    sqlx::query("UPDATE agents SET last_seen_at = $1")
        .bind(now - Duration::hours(30))
        .execute(&server.pool)
        .await
        .unwrap();
    sweep(&server, now).await;
    let rules: Vec<String> = sqlx::query_scalar("SELECT rule::text FROM alerts").fetch_all(&server.pool).await.unwrap();
    assert_eq!(rules, ["no_agent_data"]);

    // By asking, the agent has ended the silence - five minutes before the
    // sweep notices. A toast now would tell somebody at their keyboard that
    // their keyboard is quiet.
    assert_eq!(pulse(&server, &server.token).await, 0);
    assert!(kinds(&queue(&server, &server.token).await).is_empty());

    // The inbox has it: somebody away from every machine can read it there.
    let cookie = employee_session(&server).await;
    let (_, inbox) = server.get_with_cookie("/api/v1/me/notifications", cookie.as_deref()).await;
    assert!(
        inbox["notifications"].as_array().unwrap().iter().any(|n| n["alert"]["rule"] == "no_agent_data"),
        "{inbox}"
    );

    server.close().await;
}

#[tokio::test]
async fn what_the_person_read_is_not_toasted_anywhere() {
    let Some(server) = TestServer::start().await else { return };
    let now = Utc::now();
    leave_a_day_open(&server, &server.token, now, 20).await;
    sweep(&server, now).await;

    let cookie = employee_session(&server).await;
    let (_, inbox) = server.get_with_cookie("/api/v1/me/notifications", cookie.as_deref()).await;
    let newest = inbox["notifications"][0]["id"].as_i64().unwrap();
    let (status, _, cursor) = server
        .post_with_cookie("/api/v1/me/notifications/read", cookie.as_deref(), json!({ "through": newest }))
        .await;
    assert_eq!(status, StatusCode::OK, "{cursor}");
    assert_eq!(cursor["through"], newest);

    assert_eq!(pulse(&server, &server.token).await, 0, "seen in the inbox, not toasted after");
    let (_, inbox) = server.get_with_cookie("/api/v1/me/notifications", cookie.as_deref()).await;
    assert_eq!(inbox["unread"], 0);
    assert!(inbox["notifications"].as_array().unwrap().iter().all(|n| n["read"] == true), "{inbox}");

    server.close().await;
}

#[tokio::test]
async fn a_cursor_never_moves_back_and_never_past_what_exists() {
    let Some(server) = TestServer::start().await else { return };
    let now = Utc::now();
    leave_a_day_open(&server, &server.token, now, 20).await;
    sweep(&server, now).await;
    let first = queue(&server, &server.token).await["notifications"][0]["id"].as_i64().unwrap();

    // A number from the future is held to what exists. Otherwise one careless
    // constant in an agent would silence every notice not yet written.
    let (status, cursor) = ack(&server, &server.token, i64::MAX).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cursor["through"], first, "clamped to the newest notice: {cursor}");

    // A late acknowledgement, after a later one, does not bring toasts back.
    let (_, cursor) = ack(&server, &server.token, 0).await;
    assert_eq!(cursor["through"], first);

    // And the next notice still arrives.
    let admin = admin_session(&server).await;
    let (status, _, body) = server
        .put_with_cookie("/api/v1/privacy", admin.as_deref(), json!({ "level": "moderate" }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(kinds(&queue(&server, &server.token).await), ["privacy.changed"]);

    // The person's cursor is held the same way.
    let (_, cursor) = server
        .post_with_header("/api/v1/agent/notifications/read", Some(&bearer(&server.token)), json!({ "through": i64::MAX }))
        .await;
    let newest: i64 = sqlx::query_scalar("SELECT max(id) FROM notifications WHERE user_id = $1")
        .bind(user_id(&server, EMPLOYEE).await)
        .fetch_one(&server.pool)
        .await
        .unwrap();
    assert_eq!(cursor["through"], newest);

    let (status, body) = ack(&server, &server.token, -1).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    server.close().await;
}

async fn admin_session(server: &TestServer) -> Option<String> {
    server.add_admin("root@example.test", "supersecret").await;
    let (status, cookie, body) = server.login("root@example.test", "supersecret").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    cookie
}

/// Issues a second machine for the employee through the admin route, and
/// answers its id and token.
async fn second_machine(server: &TestServer, admin: Option<&str>, name: &str) -> (String, String) {
    let employee = user_id(server, EMPLOYEE).await;
    let (status, _, body) = server
        .post_with_cookie(&format!("/api/v1/users/{employee}/agents"), admin, json!({ "name": name }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    (body["id"].as_str().unwrap().to_string(), body["token"].as_str().unwrap().to_string())
}

#[tokio::test]
async fn a_new_machine_is_news_to_the_others_and_not_to_itself() {
    let Some(server) = TestServer::start().await else { return };
    let admin = admin_session(&server).await;

    let (_, laptop) = second_machine(&server, admin.as_deref(), "laptop").await;

    // The desktop hears of it - the point of the notice is the machine that
    // is not this one.
    let body = queue(&server, &server.token).await;
    assert_eq!(kinds(&body), ["agent.issued"]);
    assert_eq!(body["notifications"][0]["agent"]["name"], "laptop");
    assert_eq!(body["notifications"][0]["title"], "\u{201c}laptop\u{201d} can now report as you");

    // The laptop is not told about itself, nor about anything said before it
    // existed - the seeded desktop's own notice is older than it.
    assert_eq!(pulse(&server, &laptop).await, 0);
    assert!(kinds(&queue(&server, &laptop).await).is_empty());

    server.close().await;
}

#[tokio::test]
async fn each_machine_shows_a_notice_once_on_its_own_cursor() {
    let Some(server) = TestServer::start().await else { return };
    let admin = admin_session(&server).await;
    let (_, laptop) = second_machine(&server, admin.as_deref(), "laptop").await;
    // The desktop has shown what it had.
    let seen = queue(&server, &server.token).await["notifications"][0]["id"].as_i64().unwrap();
    ack(&server, &server.token, seen).await;

    let now = Utc::now();
    leave_a_day_open(&server, &server.token, now, 20).await;
    sweep(&server, now).await;

    // Both machines are told: the desk nobody is at must not swallow the
    // notice for the one somebody is at.
    assert_eq!(pulse(&server, &server.token).await, 1);
    assert_eq!(pulse(&server, &laptop).await, 1);

    let id = queue(&server, &laptop).await["notifications"][0]["id"].as_i64().unwrap();
    ack(&server, &laptop, id).await;
    assert_eq!(pulse(&server, &laptop).await, 0);
    assert_eq!(pulse(&server, &server.token).await, 1, "one machine's acknowledgement is not the other's");

    server.close().await;
}

#[tokio::test]
async fn a_revoked_machine_is_told_once_to_the_person() {
    let Some(server) = TestServer::start().await else { return };
    let admin = admin_session(&server).await;
    let (laptop_id, _) = second_machine(&server, admin.as_deref(), "laptop").await;

    for _ in 0..2 {
        let (status, _, body) = server.delete_with_cookie(&format!("/api/v1/agents/{laptop_id}"), admin.as_deref()).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    }

    let revoked: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind = 'agent.revoked'")
        .fetch_one(&server.pool)
        .await
        .unwrap();
    assert_eq!(revoked, 1, "a second revoke changes nothing and says nothing");
    let body = queue(&server, &server.token).await;
    assert_eq!(kinds(&body), ["agent.issued", "agent.revoked"], "oldest first");

    server.close().await;
}

#[tokio::test]
async fn a_privacy_change_is_told_to_everybody_active_and_only_when_it_changes() {
    let Some(server) = TestServer::start().await else { return };
    server.add_agent("other@example.test", "other-token").await;
    server.add_agent("gone@example.test", "gone-token").await;
    sqlx::query("UPDATE users SET active = false WHERE lower(email) = 'gone@example.test'")
        .execute(&server.pool)
        .await
        .unwrap();
    let admin = admin_session(&server).await;

    // The level it already has: nothing changed, nothing is said.
    server.put_with_cookie("/api/v1/privacy", admin.as_deref(), json!({ "level": "full" })).await;
    let before = server.count("notifications").await;

    let (status, _, body) = server.put_with_cookie("/api/v1/privacy", admin.as_deref(), json!({ "level": "coarse" })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let told: Vec<String> =
        sqlx::query_scalar("SELECT u.email FROM notifications n JOIN users u ON u.id = n.user_id WHERE n.kind = 'privacy.changed' ORDER BY u.email")
            .fetch_all(&server.pool)
            .await
            .unwrap();
    assert_eq!(
        told,
        ["employee@example.test", "other@example.test", "root@example.test"],
        "everybody active, nobody deactivated"
    );
    assert_eq!(server.count("notifications").await - before, 3);

    let notice = &queue(&server, "other-token").await["notifications"][0];
    assert_eq!(notice["privacy"], json!({ "from": "full", "to": "coarse" }));
    assert!(notice["body"].as_str().unwrap().contains("from full to coarse"), "{notice}");

    server.close().await;
}

#[tokio::test]
async fn an_inbox_is_its_owners_alone() {
    let Some(server) = TestServer::start().await else { return };
    server.add_agent("other@example.test", "other-token").await;
    let now = Utc::now();
    leave_a_day_open(&server, "other-token", now, 20).await;
    sweep(&server, now).await;

    // An administrator reads their own inbox and nobody else's: the alert
    // about `other` is on the alerts feed, where it belongs, and what `other`
    // was told is between the server and them.
    let admin = admin_session(&server).await;
    let (status, inbox) = server.get_with_cookie("/api/v1/me/notifications", admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK);
    assert!(inbox["notifications"].as_array().unwrap().is_empty(), "{inbox}");

    let cookie = employee_session(&server).await;
    let (_, inbox) = server.get_with_cookie("/api/v1/me/notifications", cookie.as_deref()).await;
    assert!(
        inbox["notifications"].as_array().unwrap().iter().all(|n| n["kind"] != "alert.raised"),
        "{inbox}"
    );

    // And one person's read cursor does not move another's.
    let (_, _, cursor) = server
        .post_with_cookie("/api/v1/me/notifications/read", cookie.as_deref(), json!({ "through": i64::MAX }))
        .await;
    let theirs: i64 = sqlx::query_scalar("SELECT max(id) FROM notifications WHERE user_id = $1")
        .bind(user_id(&server, EMPLOYEE).await)
        .fetch_one(&server.pool)
        .await
        .unwrap();
    assert_eq!(cursor["through"], theirs, "clamped to the reader's own notices");
    assert_eq!(pulse(&server, "other-token").await, 1, "other's alert is still news to other");

    // Without a session there is no inbox to read.
    let (status, _) = server.get_with_cookie("/api/v1/me/notifications", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    server.close().await;
}

#[tokio::test]
async fn a_long_queue_arrives_a_page_at_a_time() {
    let Some(server) = TestServer::start().await else { return };
    let admin = admin_session(&server).await;
    // Thirty privacy changes: more than a page, all of them news.
    for level in ["moderate", "full"].iter().cycle().take(30) {
        let (status, _, body) = server.put_with_cookie("/api/v1/privacy", admin.as_deref(), json!({ "level": level })).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    let first = queue(&server, &server.token).await;
    assert_eq!(first["notifications"].as_array().unwrap().len(), 20);
    assert_eq!(first["more"], true);
    assert_eq!(pulse(&server, &server.token).await, 30, "the count is all of them, not the page");

    let last = first["notifications"][19]["id"].as_i64().unwrap();
    ack(&server, &server.token, last).await;
    let second = queue(&server, &server.token).await;
    assert_eq!(second["notifications"].as_array().unwrap().len(), 10);
    assert_eq!(second["more"], false);
    assert!(second["notifications"][0]["id"].as_i64().unwrap() > last, "oldest first, after the cursor");

    server.close().await;
}

/// The first migration that knows about notifications. An installation on
/// 0.23.1 stands just before it.
const NOTIFICATIONS: i64 = 20260925000001;
/// The one that tells the alerts already standing when notifications arrived.
const STANDING: i64 = 20260925000002;

/// A person with one agent, written as rows: the schema here is an older one,
/// and the API that would make them is the new one.
async fn person_with_agent(db: &support::TestDb, email: &str, token: &str) -> Uuid {
    let user: Uuid = sqlx::query_scalar("INSERT INTO users (email, display_name) VALUES ($1, 'Upgraded') RETURNING id")
        .bind(email)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO agents (user_id, name, token_hash) VALUES ($1, 'desktop', $2)")
        .bind(user)
        .bind(kasl_server::auth::hash_token(token))
        .execute(&db.pool)
        .await
        .unwrap();
    user
}

#[tokio::test]
async fn alerts_standing_at_the_upgrade_are_told() {
    // An installation on 0.23.1 with alerts already raised - an open day, a
    // long day a manager has answered while it still holds, and a silence
    // that has since ended. 0.24.0 wrote notices only as alerts were raised,
    // so these three were never told: the open day, on this project's own
    // stand, was the very case the milestone exists for.
    let Some(db) = support::TestDb::create_before(NOTIFICATIONS).await else {
        return;
    };
    let user = person_with_agent(&db, "upgraded@example.test", "upgrade-token").await;
    let fired: DateTime<Utc> = "2026-09-24T09:30:00Z".parse().unwrap();
    sqlx::query(
        "INSERT INTO alerts (user_id, rule, state, observed_seconds, against_seconds, subject_date, fired_at, acknowledged_at, resolved_at)
         VALUES ($1, 'day_not_closed', 'open',         61200, 57600, '2026-09-23', $2, NULL, NULL),
                ($1, 'overwork',       'acknowledged', 45000, 28800, '2026-09-22', $2 + interval '1 minute', $2, NULL),
                ($1, 'no_agent_data',  'resolved',     50000, 43200, NULL,         $2 - interval '1 day', NULL, $2)",
    )
    .bind(user)
    .bind(fired)
    .execute(&db.pool)
    .await
    .unwrap();

    db.upgrade().await;

    // Read back through the agent's own route, which is the proof that the
    // payload the migration built is the shape the server reads.
    let server = TestServer::wrap(db);
    let body = queue(&server, "upgrade-token").await;
    let notices = body["notifications"].as_array().unwrap();
    assert_eq!(notices.len(), 2, "what stands is told and what resolved is not: {body}");
    assert_eq!(notices[0]["alert"]["rule"], "day_not_closed", "in the order the alerts fired");
    assert_eq!(notices[0]["alert"]["subject_date"], "2026-09-23");
    assert_eq!(notices[0]["alert"]["observed_seconds"], 61200, "the figures it fired on");
    assert_eq!(notices[0]["title"], "Your day of 2026-09-23 is still open here");
    assert_eq!(notices[1]["alert"]["rule"], "overwork");
    assert_eq!(notices[1]["body"], "You worked 12.5 h that day, against your norm of 8 h.");

    // And the one that resolved is not written at all. The queue above would
    // hide it anyway - an alert that is over is not toasted - so only the
    // table can say it was never told.
    assert_eq!(server.count("notifications").await, 2, "a silence that already ended is not news");

    server.close().await;
}

#[tokio::test]
async fn an_alert_already_told_is_not_told_again_by_the_upgrade() {
    // An installation that ran 0.24.0 has notices for the alerts it raised.
    // The migration that tells the standing ones must not add a second.
    let Some(db) = support::TestDb::create_before(STANDING).await else { return };
    let user = person_with_agent(&db, "upgraded@example.test", "upgrade-token").await;
    let alert: kasl_server::webhooks::AlertPayload = sqlx::query_as(
        "INSERT INTO alerts (user_id, rule, observed_seconds, against_seconds, subject_date)
         VALUES ($1, 'day_not_closed', 61200, 57600, '2026-09-23')
         RETURNING id, rule, observed_seconds, against_seconds, subject_date, fired_at",
    )
    .bind(user)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let mut conn = db.pool.acquire().await.unwrap();
    kasl_server::notifications::alert_raised(&mut conn, user, &alert).await.unwrap();
    drop(conn);

    db.upgrade().await;

    let told: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE alert_id = $1")
        .bind(alert.id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(told, 1);

    TestServer::wrap(db).close().await;
}
