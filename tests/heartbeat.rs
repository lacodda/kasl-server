//! The pulse and the live dashboard, against a live database.
//!
//! Two things are worth proving here, and neither is "the endpoint answers".
//! The first is that silence is never read as a claim: an agent that stopped
//! sending, and one that never sent anything, must not appear as somebody at
//! work. The second is the permission - `/team/live` says who is at their
//! keyboard right now, which is more sensitive than a week's totals, so it has
//! to obey the same visibility rule as everything else in `team`.

mod support;

use axum::http::StatusCode;
use chrono::{SecondsFormat, Utc};
use serde_json::{Value, json};
use support::TestServer;

/// A pulse stamped now, in the shape an agent sends.
fn pulse(state: &str) -> Value {
    json!({ "state": state, "at": Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true) })
}

/// The live row for one user id.
fn row<'a>(body: &'a Value, user_id: &str) -> &'a Value {
    body["members"]
        .as_array()
        .expect("members")
        .iter()
        .find(|member| member["user_id"] == user_id)
        .expect("the person should be listed")
}

/// An admin, plus one employee with an agent. Returns the admin's cookie, the
/// employee's token, and the employee's id.
async fn one_employee(server: &TestServer) -> (Option<String>, String, String) {
    server.add_admin("boss@example.test", "correct horse").await;
    let (_, admin, _) = server.login("boss@example.test", "correct horse").await;
    let token = server.add_agent("kirill@example.test", "token-kirill").await;

    let (status, body) = server.get_with_cookie("/api/v1/users", admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = body
        .as_array()
        .expect("a list of users")
        .iter()
        .find(|user| user["email"] == "kirill@example.test")
        .and_then(|user| user["id"].as_str())
        .expect("the employee should exist")
        .to_string();

    (admin, token, id)
}

#[tokio::test]
async fn a_pulse_reaches_the_dashboard() {
    let Some(server) = TestServer::start().await else { return };
    let (admin, token, id) = one_employee(&server).await;

    let (status, body) = server
        .post_with_header("/api/v1/agent/heartbeat", Some(&format!("Bearer {token}")), pulse("working"))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    // The agent is told the cadence rather than choosing one: the interval and
    // the staleness threshold have to agree, and only the server knows both.
    assert!(body["interval_seconds"].as_i64().expect("an interval") > 0, "{body}");
    assert!(
        body["stale_after_seconds"].as_i64().expect("a threshold") > body["interval_seconds"].as_i64().unwrap(),
        "an agent must be able to miss a pulse: {body}"
    );
    assert_eq!(body["state"], "working");

    let (status, body) = server.get_with_cookie("/api/v1/team/live", admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(row(&body, &id)["status"], "working", "{body}");

    server.close().await;
}

#[tokio::test]
async fn the_latest_pulse_replaces_the_one_before_it() {
    let Some(server) = TestServer::start().await else { return };
    let (admin, token, id) = one_employee(&server).await;

    let authorization = format!("Bearer {token}");
    server.post_with_header("/api/v1/agent/heartbeat", Some(&authorization), pulse("working")).await;
    server.post_with_header("/api/v1/agent/heartbeat", Some(&authorization), pulse("paused")).await;

    let (_, body) = server.get_with_cookie("/api/v1/team/live", admin.as_deref()).await;
    assert_eq!(row(&body, &id)["status"], "paused", "{body}");

    // One row per agent, not a log: the pulse arrives every minute per person,
    // and a table that grew a row each time would be the largest thing in the
    // installation within a week - and a minute-by-minute record of when
    // somebody was at their desk, which this product does not keep.
    let rows: i64 = server
        .scalar("SELECT count(*) FROM agents a JOIN users u ON u.id = a.user_id WHERE u.email = 'kirill@example.test'")
        .await;
    assert_eq!(rows, 1, "the pulse must overwrite, not accumulate");

    server.close().await;
}

#[tokio::test]
async fn an_agent_that_never_sent_a_pulse_is_unknown_rather_than_idle() {
    let Some(server) = TestServer::start().await else { return };
    let (admin, _, id) = one_employee(&server).await;

    // The regression this guards: reading silence as a state. Every agent
    // shipped before this endpoint existed sends nothing here, and showing
    // that whole population as "not in a day" would be the server inventing
    // an answer it does not have.
    let (status, body) = server.get_with_cookie("/api/v1/team/live", admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(row(&body, &id)["status"], "unknown", "{body}");
    assert_eq!(row(&body, &id)["since_received"], Value::Null, "{body}");

    server.close().await;
}

#[tokio::test]
async fn a_stale_pulse_reads_as_offline() {
    let Some(server) = TestServer::start().await else { return };
    let (admin, token, id) = one_employee(&server).await;

    server
        .post_with_header("/api/v1/agent/heartbeat", Some(&format!("Bearer {token}")), pulse("working"))
        .await;
    // Age it past the threshold rather than waiting three minutes. What is
    // under test is that the reading is derived from the receipt time at all:
    // an agent killed mid-day must stop being shown as working, instead of
    // leaving its last claim on the dashboard indefinitely.
    server.execute("UPDATE agents SET heartbeat_received_at = now() - interval '1 day'").await;

    let (_, body) = server.get_with_cookie("/api/v1/team/live", admin.as_deref()).await;
    assert_eq!(row(&body, &id)["status"], "offline", "{body}");
    assert!(row(&body, &id)["since_received"].as_i64().expect("an age") > 3600, "{body}");

    server.close().await;
}

#[tokio::test]
async fn a_revoked_agent_stops_being_evidence() {
    let Some(server) = TestServer::start().await else { return };
    let (admin, token, id) = one_employee(&server).await;

    server
        .post_with_header("/api/v1/agent/heartbeat", Some(&format!("Bearer {token}")), pulse("working"))
        .await;
    server.execute("UPDATE agents SET revoked_at = now()").await;

    // Revocation has to reach the dashboard, not only the routes that write.
    // A withdrawn agent whose last words keep a person marked "working" is a
    // revocation that did not finish.
    let (_, body) = server.get_with_cookie("/api/v1/team/live", admin.as_deref()).await;
    assert_eq!(row(&body, &id)["status"], "unknown", "{body}");

    server.close().await;
}

#[tokio::test]
async fn a_pulse_from_the_future_is_refused() {
    let Some(server) = TestServer::start().await else { return };
    let (admin, token, id) = one_employee(&server).await;

    let ahead = Utc::now() + chrono::Duration::hours(2);
    let (status, body) = server
        .post_with_header(
            "/api/v1/agent/heartbeat",
            Some(&format!("Bearer {token}")),
            json!({ "state": "working", "at": ahead.to_rfc3339_opts(SecondsFormat::Secs, true) }),
        )
        .await;

    // Refused rather than clamped: a stamp hours ahead would sit "fresh" for
    // as long as the skew lasts, which is how a machine with a broken clock
    // could look alive all day. The agent is told, so a person can fix it.
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().expect("a message").contains("clock"), "{body}");

    let (_, body) = server.get_with_cookie("/api/v1/team/live", admin.as_deref()).await;
    assert_eq!(row(&body, &id)["status"], "unknown", "a refused pulse must not be stored: {body}");

    server.close().await;
}

#[tokio::test]
async fn an_unknown_token_cannot_send_a_pulse() {
    let Some(server) = TestServer::start().await else { return };
    let (_, _, _) = one_employee(&server).await;

    let (status, _) = server
        .post_with_header("/api/v1/agent/heartbeat", Some("Bearer not-a-real-token"), pulse("working"))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = server.post_with_header("/api/v1/agent/heartbeat", None, pulse("working")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    server.close().await;
}

#[tokio::test]
async fn an_employee_cannot_read_the_team_feed() {
    let Some(server) = TestServer::start().await else { return };
    server.add_admin("boss@example.test", "correct horse").await;
    server.add_agent("kirill@example.test", "token-kirill").await;
    server.set_password("kirill@example.test", "correct horse").await;
    let (_, employee, _) = server.login("kirill@example.test", "correct horse").await;

    // Who is at their keyboard right now is a manager's question. An employee
    // reading it would learn more about their colleagues live than the week's
    // totals ever tell them.
    let (status, _) = server.get_with_cookie("/api/v1/team/live", employee.as_deref()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _) = server.get_with_cookie("/api/v1/team/live", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    server.close().await;
}

#[tokio::test]
async fn the_feed_lists_only_people_the_reader_may_see() {
    let Some(server) = TestServer::start().await else { return };
    server.add_admin("boss@example.test", "correct horse").await;
    let (_, admin, _) = server.login("boss@example.test", "correct horse").await;

    server.add_agent("inside@example.test", "inside-token").await;
    server.add_agent("outside@example.test", "outside-token").await;
    server.add_agent("lead@example.test", "lead-token").await;
    for email in ["inside@example.test", "outside@example.test", "lead@example.test"] {
        server.set_password(email, "correct horse").await;
    }

    let (_, users) = server.get_with_cookie("/api/v1/users", admin.as_deref()).await;
    let id_of = |email: &str| {
        users
            .as_array()
            .expect("users")
            .iter()
            .find(|user| user["email"] == email)
            .and_then(|user| user["id"].as_str())
            .expect("the account should exist")
            .to_string()
    };
    let (lead_id, inside_id, outside_id) = (id_of("lead@example.test"), id_of("inside@example.test"), id_of("outside@example.test"));

    server
        .patch_with_cookie(&format!("/api/v1/users/{lead_id}"), admin.as_deref(), json!({ "role": "manager" }))
        .await;
    let (_, _, department) = server
        .post_with_cookie("/api/v1/departments", admin.as_deref(), json!({ "name": "Engineering", "manager_id": lead_id }))
        .await;
    let department_id = department["id"].as_str().expect("an id").to_string();
    server
        .put_with_cookie(
            &format!("/api/v1/users/{inside_id}/department"),
            admin.as_deref(),
            json!({ "department_id": department_id }),
        )
        .await;

    server
        .post_with_header("/api/v1/agent/heartbeat", Some("Bearer outside-token"), pulse("working"))
        .await;

    let (_, manager, _) = server.login("lead@example.test", "correct horse").await;
    let (status, body) = server.get_with_cookie("/api/v1/team/live", manager.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let listed: Vec<&str> = body["members"]
        .as_array()
        .expect("members")
        .iter()
        .map(|member| member["user_id"].as_str().expect("an id"))
        .collect();
    assert!(listed.contains(&lead_id.as_str()), "a manager sees themselves: {body}");
    assert!(listed.contains(&inside_id.as_str()), "and their department: {body}");
    // The whole point: the live feed applies the same visibility rule as the
    // rest of `team`. Someone else's department working at this moment is not
    // this manager's business, and a leak here is invisible to its victim.
    assert!(!listed.contains(&outside_id.as_str()), "and nobody else: {body}");

    server.close().await;
}
