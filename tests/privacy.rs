//! The privacy manifest, against a live database.
//!
//! The claim under test is not "the endpoint answers" but "what the level
//! excludes is not in the database" (ADR 0011). So every level test reads rows
//! back rather than trusting the response that says what it did - a handler
//! could report a pause as discarded and store it anyway, and only the table
//! knows which happened.

mod support;

use axum::http::StatusCode;
use serde_json::{Value, json};
use support::TestServer;

/// A day with everything a level could take away: a pause with a reason typed
/// into it, and a task with a comment.
fn detailed_day(date: &str) -> Value {
    json!({
        "date": date,
        "started_at": format!("{date}T09:00:00-03:00"),
        "ended_at": format!("{date}T18:00:00-03:00"),
        "pauses": [
            {
                "started_at": format!("{date}T12:00:00-03:00"),
                "ended_at": format!("{date}T12:30:00-03:00"),
                "duration_seconds": 1800,
                "manual": true,
                "reason": "doctor"
            },
            {
                "started_at": format!("{date}T15:00:00-03:00"),
                "ended_at": format!("{date}T15:10:00-03:00"),
                "duration_seconds": 600,
                "manual": false
            }
        ],
        "tasks": [
            {
                "agent_task_id": 1,
                "recorded_at": format!("{date}T17:00:00-03:00"),
                "name": "Ship the release",
                "comment": "took longer than planned",
                "completeness": 100
            }
        ]
    })
}

/// Sets the level the way an administrator would, through the API.
async fn set_level(server: &TestServer, level: &str) -> (StatusCode, Value) {
    server.add_admin("boss@example.test", "correct horse").await;
    let (_, cookie, _) = server.login("boss@example.test", "correct horse").await;
    let (status, _, body) = server.put_with_cookie("/api/v1/privacy", cookie.as_deref(), json!({ "level": level })).await;
    (status, body)
}

#[tokio::test]
async fn the_default_level_stores_everything_the_agent_sent() {
    let Some(server) = TestServer::start().await else { return };

    let (status, body) = server.post_day(&server.token, detailed_day("2026-08-24")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["privacy_level"], "full");
    // Nothing discarded, so the field is absent rather than a row of zeroes.
    assert!(body.get("discarded").is_none(), "an untouched day should not report discards: {body}");

    let (reason, comment): (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT (SELECT reason FROM pauses p JOIN workdays w ON w.id = p.workday_id WHERE w.date = '2026-08-24' AND p.manual),
                (SELECT comment FROM tasks WHERE date = '2026-08-24')",
    )
    .fetch_one(&server.pool)
    .await
    .expect("the query should run");

    assert_eq!(reason.as_deref(), Some("doctor"), "full keeps the reason");
    assert_eq!(comment.as_deref(), Some("took longer than planned"), "full keeps the comment");
}

#[tokio::test]
async fn moderate_keeps_the_times_and_drops_the_words() {
    let Some(server) = TestServer::start().await else { return };
    let (status, _) = set_level(&server, "moderate").await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = server.post_day(&server.token, detailed_day("2026-08-24")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["privacy_level"], "moderate");
    // One pause reason and one task comment.
    assert_eq!(body["discarded"]["free_text"], 2, "{body}");
    assert!(body["discarded"].get("pauses").is_none(), "moderate keeps the pauses themselves: {body}");

    // The rows are there; the words are not.
    let pauses: i64 = sqlx::query_scalar("SELECT count(*) FROM pauses p JOIN workdays w ON w.id = p.workday_id WHERE w.date = '2026-08-24'")
        .fetch_one(&server.pool)
        .await
        .expect("the query should run");
    assert_eq!(pauses, 2, "moderate stores every pause");

    let reasons: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pauses p JOIN workdays w ON w.id = p.workday_id WHERE w.date = '2026-08-24' AND p.reason IS NOT NULL")
            .fetch_one(&server.pool)
            .await
            .expect("the query should run");
    assert_eq!(reasons, 0, "no reason should have reached the table");

    let comments: i64 = sqlx::query_scalar("SELECT count(*) FROM tasks WHERE date = '2026-08-24' AND comment IS NOT NULL")
        .fetch_one(&server.pool)
        .await
        .expect("the query should run");
    assert_eq!(comments, 0, "no comment should have reached the table");

    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM tasks WHERE date = '2026-08-24'")
        .fetch_all(&server.pool)
        .await
        .expect("the query should run");
    assert_eq!(names, vec!["Ship the release"], "moderate keeps task names");
}

#[tokio::test]
async fn coarse_stores_hours_and_a_summary_instead_of_a_timeline() {
    let Some(server) = TestServer::start().await else { return };
    set_level(&server, "coarse").await;

    let (status, body) = server.post_day(&server.token, detailed_day("2026-08-24")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["privacy_level"], "coarse");
    assert_eq!(body["discarded"]["pauses"], 2, "{body}");
    assert_eq!(body["discarded"]["tasks"], 1, "{body}");

    let pauses: i64 = sqlx::query_scalar("SELECT count(*) FROM pauses p JOIN workdays w ON w.id = p.workday_id WHERE w.date = '2026-08-24'")
        .fetch_one(&server.pool)
        .await
        .expect("the query should run");
    assert_eq!(pauses, 0, "coarse stores no individual pauses");

    let tasks: i64 = sqlx::query_scalar("SELECT count(*) FROM tasks WHERE date = '2026-08-24'")
        .fetch_one(&server.pool)
        .await
        .expect("the query should run");
    assert_eq!(tasks, 0, "coarse stores no tasks");

    // The day must not read as uninterrupted work: the summary is what keeps
    // its hours honest once the rows are gone.
    let (count, seconds): (Option<i32>, Option<i32>) = sqlx::query_as("SELECT paused_count, paused_seconds FROM workdays WHERE date = '2026-08-24'")
        .fetch_one(&server.pool)
        .await
        .expect("the query should run");
    assert_eq!(count, Some(2), "the day should say how many times it was interrupted");
    assert_eq!(seconds, Some(2400), "30 minutes plus 10");
}

#[tokio::test]
async fn a_day_the_agent_resends_is_stored_under_the_policy_in_force_now() {
    let Some(server) = TestServer::start().await else { return };

    // Sent while the installation kept everything.
    let (status, _) = server.post_day(&server.token, detailed_day("2026-08-24")).await;
    assert_eq!(status, StatusCode::OK);

    set_level(&server, "coarse").await;

    // The same day again - a correction, or a retry after a lost connection.
    let (status, body) = server.post_day(&server.token, detailed_day("2026-08-24")).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // The pause rows the wider policy had written are replaced by nothing:
    // `replace_pauses` clears the day before writing what the level allowed.
    let pauses: i64 = sqlx::query_scalar("SELECT count(*) FROM pauses p JOIN workdays w ON w.id = p.workday_id WHERE w.date = '2026-08-24'")
        .fetch_one(&server.pool)
        .await
        .expect("the query should run");
    assert_eq!(pauses, 0, "a re-sent day is stored under the level in force now");

    let (count, seconds): (Option<i32>, Option<i32>) = sqlx::query_as("SELECT paused_count, paused_seconds FROM workdays WHERE date = '2026-08-24'")
        .fetch_one(&server.pool)
        .await
        .expect("the query should run");
    assert_eq!((count, seconds), (Some(2), Some(2400)), "the summary replaces the rows it dropped");

    // The task written while the policy was wider must go too, and it does not
    // ride on `tasks_are_complete` - the day above never sets it. Pauses are
    // replaced wholesale and so looked correct on their own; the task is what
    // a real run through the server exposed.
    let tasks: i64 = sqlx::query_scalar("SELECT count(*) FROM tasks WHERE date = '2026-08-24'")
        .fetch_one(&server.pool)
        .await
        .expect("the query should run");
    assert_eq!(tasks, 0, "a level that stores no tasks must not keep the ones a wider level left");
}

#[tokio::test]
async fn widening_the_level_does_not_bring_back_what_was_dropped() {
    let Some(server) = TestServer::start().await else { return };
    set_level(&server, "coarse").await;

    let (status, _) = server.post_day(&server.token, detailed_day("2026-08-24")).await;
    assert_eq!(status, StatusCode::OK);

    set_level(&server, "full").await;

    // Nothing re-sent, so nothing returns: what ingest discarded was never
    // written, and no setting can recover it. The manifest says so, and this
    // is the test that it is true.
    let tasks: i64 = sqlx::query_scalar("SELECT count(*) FROM tasks WHERE date = '2026-08-24'")
        .fetch_one(&server.pool)
        .await
        .expect("the query should run");
    assert_eq!(tasks, 0, "widening the policy cannot restore a dropped task");
}

#[tokio::test]
async fn a_batch_reads_the_policy_once_and_applies_it_to_every_day() {
    let Some(server) = TestServer::start().await else { return };
    set_level(&server, "coarse").await;

    let (status, body) = server
        .post_batch(&server.token, json!([detailed_day("2026-08-24"), detailed_day("2026-08-25")]))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["accepted"], 2);

    for result in body["results"].as_array().expect("results should be an array") {
        assert_eq!(result["privacy_level"], "coarse", "{result}");
        assert_eq!(result["discarded"]["tasks"], 1, "{result}");
    }

    let tasks: i64 = sqlx::query_scalar("SELECT count(*) FROM tasks")
        .fetch_one(&server.pool)
        .await
        .expect("the query should run");
    assert_eq!(tasks, 0, "no day in the batch may slip past the policy");
}

#[tokio::test]
async fn an_employee_reads_the_manifest_and_cannot_change_it() {
    let Some(server) = TestServer::start().await else { return };
    // The seeded agent's account is an ordinary employee.
    server.set_password("employee@example.test", "correct horse").await;
    let (_, cookie, _) = server.login("employee@example.test", "correct horse").await;

    let (status, body) = server.get_with_cookie("/api/v1/privacy", cookie.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["level"], "full");
    assert!(body["summary"].as_str().expect("a summary").contains("working hours"), "{body}");
    assert!(
        body["never_collected"]
            .as_array()
            .expect("a list")
            .iter()
            .any(|item| item == "keystrokes or what you type"),
        "the manifest must name what is never collected: {body}"
    );

    let (status, _, _) = server.put_with_cookie("/api/v1/privacy", cookie.as_deref(), json!({ "level": "coarse" })).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "an employee cannot set the policy");
}

#[tokio::test]
async fn the_manifest_is_refused_to_a_stranger() {
    let Some(server) = TestServer::start().await else { return };

    let (status, _) = server.get_with_cookie("/api/v1/privacy", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "the manifest is for this installation's people");

    let (status, _) = server.get_with_header("/api/v1/privacy/agent", Some("Bearer not-a-token")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn an_agent_reads_the_manifest_with_its_own_token() {
    let Some(server) = TestServer::start().await else { return };
    set_level(&server, "moderate").await;

    // The point of the route: kasl can show this in the CLI without the
    // employee signing into the server that watches them.
    let (status, body) = server.get_with_header("/api/v1/privacy/agent", Some(&format!("Bearer {}", server.token))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["level"], "moderate");
    assert!(
        !body["stored"].as_array().expect("a list").iter().any(|item| item["what"] == "pause reasons"),
        "moderate must not promise to store reasons: {body}"
    );
}

#[tokio::test]
async fn changing_the_level_is_recorded_with_both_ends_of_the_change() {
    let Some(server) = TestServer::start().await else { return };
    set_level(&server, "coarse").await;

    let (action, details): (String, Value) = sqlx::query_as("SELECT action, details FROM audit_log WHERE action = 'privacy.level_changed'")
        .fetch_one(&server.pool)
        .await
        .expect("the change should be recorded");

    assert_eq!(action, "privacy.level_changed");
    // Both ends: "someone changed the policy" without saying from what is not
    // enough to tell a tightening from a loosening.
    assert_eq!(details["from"], "full", "{details}");
    assert_eq!(details["to"], "coarse", "{details}");
}

#[tokio::test]
async fn an_unknown_level_is_refused_rather_than_guessed() {
    let Some(server) = TestServer::start().await else { return };
    server.add_admin("boss@example.test", "correct horse").await;
    let (_, cookie, _) = server.login("boss@example.test", "correct horse").await;

    let (status, _, _) = server
        .put_with_cookie("/api/v1/privacy", cookie.as_deref(), json!({ "level": "none-of-your-business" }))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "a typo must not silently leave the policy as it was");

    let level: String = sqlx::query_scalar("SELECT privacy_level::text FROM settings WHERE singleton")
        .fetch_one(&server.pool)
        .await
        .expect("the query should run");
    assert_eq!(level, "full", "the policy should be untouched by a refused request");
}
