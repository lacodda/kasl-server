//! Exercises `POST /api/v1/days` end to end: a real router, a real database.
//!
//! This is the contract kasl agents will hold the server to, so the tests are
//! written from the agent's side - send a payload, read the status and body,
//! then check what the dashboards would find in the tables. Unit tests already
//! cover parsing and validation; what needs a database is the part that only
//! shows up once rows exist: a re-upload correcting a day instead of doubling
//! it, one agent being unable to write into another's history.
//!
//! Skipped unless `DATABASE_URL` is set; CI runs them with a Postgres service.

mod support;

use axum::http::StatusCode;
use serde_json::{Value, json};
use support::TestServer;

/// A full day, the shape an agent sends at the end of a working day.
fn a_day() -> Value {
    json!({
        "date": "2026-08-14",
        "started_at": "2026-08-14T09:00:00-03:00",
        "ended_at": "2026-08-14T18:00:00-03:00",
        "pauses": [
            {"started_at": "2026-08-14T13:00:00-03:00", "ended_at": "2026-08-14T14:00:00-03:00", "duration_seconds": 3600, "manual": true, "reason": "lunch"},
            {"started_at": "2026-08-14T16:00:00-03:00", "ended_at": "2026-08-14T16:12:00-03:00", "duration_seconds": 720}
        ],
        "tasks": [
            {"agent_task_id": 1, "recorded_at": "2026-08-14T17:50:00-03:00", "name": "Write the ingest endpoint", "completeness": 80},
            {"agent_task_id": 2, "agent_group_id": 1, "recorded_at": "2026-08-14T17:55:00-03:00", "name": "Review the schema", "comment": "with Kirill", "completeness": 100}
        ]
    })
}

#[tokio::test]
async fn a_day_lands_whole() {
    let Some(server) = TestServer::start().await else { return };

    let (status, body) = server.post_day(&server.token, a_day()).await;
    assert_eq!(status, StatusCode::OK, "a well-formed day should be accepted: {body}");
    assert_eq!(body["date"], "2026-08-14");
    assert_eq!(body["pauses"], 2);
    assert_eq!(body["tasks"], 2);

    assert_eq!(server.count("workdays").await, 1);
    assert_eq!(server.count("pauses").await, 2);
    assert_eq!(server.count("tasks").await, 2);

    // The offset the agent sent is what makes the instant absolute: 09:00-03:00
    // is 12:00 UTC, and that is what every reader must see.
    let started_at: String = server.scalar("SELECT to_char(started_at AT TIME ZONE 'UTC', 'HH24:MI') FROM workdays").await;
    assert_eq!(started_at, "12:00", "the agent's offset must be applied, not discarded");

    // The employee's own date is stored as sent, not derived from the instant.
    let date: String = server.scalar("SELECT to_char(date, 'YYYY-MM-DD') FROM workdays").await;
    assert_eq!(date, "2026-08-14");

    let manual: bool = server.scalar("SELECT manual FROM pauses WHERE reason = 'lunch'").await;
    assert!(manual, "a break the employee entered stays marked as theirs");

    // An absent agent_group_id means "this task belongs to itself".
    let group: i32 = server.scalar("SELECT agent_group_id FROM tasks WHERE agent_task_id = 1").await;
    assert_eq!(group, 1);
}

#[tokio::test]
async fn re_uploading_a_day_corrects_it_instead_of_duplicating() {
    let Some(server) = TestServer::start().await else { return };

    server.post_day(&server.token, a_day()).await;

    // The employee fixed the day in kasl: the first task is finished, the
    // short pause turned out not to be one, and they worked an hour longer.
    let mut corrected = a_day();
    corrected["ended_at"] = json!("2026-08-14T19:00:00-03:00");
    corrected["pauses"] = json!([
        {"started_at": "2026-08-14T13:00:00-03:00", "ended_at": "2026-08-14T14:00:00-03:00", "duration_seconds": 3600, "manual": true, "reason": "lunch"}
    ]);
    corrected["tasks"][0]["completeness"] = json!(100);

    let (status, _) = server.post_day(&server.token, corrected).await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(server.count("workdays").await, 1, "the same date must not become a second day");
    assert_eq!(server.count("pauses").await, 1, "a pause the employee removed must not survive");
    assert_eq!(server.count("tasks").await, 2, "tasks are matched by the agent's id, not replaced");

    let completeness: i16 = server.scalar("SELECT completeness FROM tasks WHERE agent_task_id = 1").await;
    assert_eq!(completeness, 100, "the correction must win: the agent is the source of truth");

    let ended_at: String = server.scalar("SELECT to_char(ended_at AT TIME ZONE 'UTC', 'HH24:MI') FROM workdays").await;
    assert_eq!(ended_at, "22:00", "19:00-03:00 is 22:00 UTC");
}

#[tokio::test]
async fn sending_the_same_payload_twice_changes_nothing() {
    let Some(server) = TestServer::start().await else { return };

    // What a retry after a lost connection looks like: the agent cannot know
    // whether the first attempt landed, so sending again must be safe.
    server.post_day(&server.token, a_day()).await;
    let (status, _) = server.post_day(&server.token, a_day()).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(server.count("workdays").await, 1);
    assert_eq!(server.count("pauses").await, 2);
    assert_eq!(server.count("tasks").await, 2);
}

#[tokio::test]
async fn a_days_tasks_carry_across_dates_without_colliding() {
    let Some(server) = TestServer::start().await else { return };

    server.post_day(&server.token, a_day()).await;

    // The same task, still open, continues into the next day.
    let next = json!({
        "date": "2026-08-15",
        "started_at": "2026-08-15T09:00:00-03:00",
        "tasks": [{"agent_task_id": 1, "recorded_at": "2026-08-15T10:00:00-03:00", "name": "Write the ingest endpoint", "completeness": 95}]
    });
    let (status, _) = server.post_day(&server.token, next).await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(server.count("workdays").await, 2, "a new date is a new day");
    assert_eq!(server.count("pauses").await, 2, "yesterday's pauses are untouched");
    assert_eq!(server.count("tasks").await, 2, "the carried task moved rather than multiplied");

    let date: String = server.scalar("SELECT to_char(date, 'YYYY-MM-DD') FROM tasks WHERE agent_task_id = 1").await;
    assert_eq!(date, "2026-08-15", "the task now belongs to the day it was last worked on");
}

#[tokio::test]
async fn an_upload_without_a_valid_token_is_refused() {
    let Some(server) = TestServer::start().await else { return };

    for (label, header) in [
        ("no header at all", None),
        ("an unknown token", Some("Bearer not-a-real-token".to_string())),
        ("another scheme", Some("Basic dXNlcjpwYXNz".to_string())),
    ] {
        let (status, body) = server.post_day_with_header(header.as_deref(), a_day()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{label} must not be accepted");
        assert!(body["error"].is_string(), "{label}: the refusal should explain itself");
    }

    assert_eq!(server.count("workdays").await, 0, "nothing may be written without credentials");
}

#[tokio::test]
async fn a_revoked_token_stops_working() {
    let Some(server) = TestServer::start().await else { return };

    server.execute("UPDATE agents SET revoked_at = now()").await;

    let (status, _) = server.post_day(&server.token, a_day()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "a withdrawn token must stop being accepted");
    assert_eq!(server.count("workdays").await, 0);
}

#[tokio::test]
async fn a_deactivated_employee_stops_reporting() {
    let Some(server) = TestServer::start().await else { return };

    server.execute("UPDATE users SET active = false").await;

    let (status, _) = server.post_day(&server.token, a_day()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "a departed employee's agent must stop being accepted");
}

#[tokio::test]
async fn an_agent_writes_only_into_its_own_history() {
    let Some(server) = TestServer::start().await else { return };
    let colleague = server.add_agent("colleague@example.test", "colleague-token").await;

    server.post_day(&server.token, a_day()).await;
    server.post_day(&colleague, a_day()).await;

    // Same date, two people: two days, each owned by its reporter.
    assert_eq!(server.count("workdays").await, 2);
    let mine: i64 = server
        .scalar("SELECT count(*) FROM workdays w JOIN users u ON u.id = w.user_id WHERE u.email = 'colleague@example.test'")
        .await;
    assert_eq!(mine, 1, "the colleague's upload belongs to the colleague");
}

#[tokio::test]
async fn a_malformed_day_is_refused_with_a_reason() {
    let Some(server) = TestServer::start().await else { return };

    // The contract's core requirement: an instant without an offset - which is
    // exactly what kasl stores locally - must not be accepted silently.
    let mut bare = a_day();
    bare["started_at"] = json!("2026-08-14T09:00:00");
    let (status, _) = server.post_day(&server.token, bare).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "wall-clock time without an offset must be refused");

    let mut backwards = a_day();
    backwards["ended_at"] = json!("2026-08-14T08:00:00-03:00");
    let (status, body) = server.post_day(&server.token, backwards).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("ended_at"), "the reason should name the field: {body}");

    let mut impossible = a_day();
    impossible["tasks"][0]["completeness"] = json!(140);
    let (status, body) = server.post_day(&server.token, impossible).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"].as_str().unwrap().contains("tasks[0]"),
        "the reason should point at the element: {body}"
    );

    assert_eq!(server.count("workdays").await, 0, "a refused day must leave nothing behind");
}

#[tokio::test]
async fn a_failed_day_is_not_half_written() {
    let Some(server) = TestServer::start().await else { return };

    server.post_day(&server.token, a_day()).await;

    // A failure the validator cannot see, only the database: a task name past
    // the column's limit. By the time it is raised, the day has been updated
    // and its pauses deleted and reinserted - so without a transaction the
    // upload would leave the day mangled and report an error at the same time.
    server
        .execute("ALTER TABLE tasks ADD CONSTRAINT tasks_name_length CHECK (length(name) <= 40)")
        .await;

    let mut broken = a_day();
    broken["pauses"] = json!([{"started_at": "2026-08-14T13:00:00-03:00", "ended_at": "2026-08-14T14:00:00-03:00", "duration_seconds": 3600}]);
    broken["ended_at"] = json!("2026-08-14T23:00:00-03:00");
    broken["tasks"][1]["name"] = json!("x".repeat(60));

    let (status, body) = server.post_day(&server.token, broken).await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "a payload the database refuses must not report success: {body}"
    );

    // Everything the failed upload touched must look untouched.
    assert_eq!(server.count("pauses").await, 2, "the original pauses must survive a failed upload");
    assert_eq!(server.count("tasks").await, 2, "and so must the original tasks");
    let ended_at: String = server.scalar("SELECT to_char(ended_at AT TIME ZONE 'UTC', 'HH24:MI') FROM workdays").await;
    assert_eq!(ended_at, "21:00", "the day must keep the end time of the last upload that succeeded");
}

#[tokio::test]
async fn an_open_day_is_accepted() {
    let Some(server) = TestServer::start().await else { return };

    // What the agent sends mid-day: no end, a pause still running.
    let open = json!({
        "date": "2026-08-14",
        "started_at": "2026-08-14T09:00:00-03:00",
        "pauses": [{"started_at": "2026-08-14T13:00:00-03:00"}],
        "tasks": []
    });

    let (status, _) = server.post_day(&server.token, open).await;
    assert_eq!(status, StatusCode::OK, "a day in progress is a normal thing to report");

    let ended: Option<String> = server.optional_scalar("SELECT to_char(ended_at, 'HH24:MI') FROM workdays").await;
    assert!(ended.is_none(), "an open day has no end yet");
    let pause_end: Option<String> = server.optional_scalar("SELECT to_char(ended_at, 'HH24:MI') FROM pauses").await;
    assert!(pause_end.is_none(), "a running pause has no end yet");
}

#[tokio::test]
async fn a_task_deleted_on_the_agent_is_deleted_here() {
    let Some(server) = TestServer::start().await else { return };

    server.post_day(&server.token, a_day()).await;
    assert_eq!(server.count("tasks").await, 2);

    // The employee deleted task 2 in kasl; the agent re-sends the day with what
    // is left and declares the list complete.
    let mut corrected = a_day();
    corrected["tasks"] = json!([{"agent_task_id": 1, "recorded_at": "2026-08-14T17:50:00-03:00", "name": "Write the ingest endpoint", "completeness": 80}]);
    corrected["tasks_are_complete"] = json!(true);

    let (status, body) = server.post_day(&server.token, corrected).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted_tasks"], 1, "the server should report what it dropped");

    assert_eq!(server.count("tasks").await, 1, "the deleted task must be gone");
    let survivor: i32 = server.scalar("SELECT agent_task_id FROM tasks").await;
    assert_eq!(survivor, 1, "and the one still sent must be the survivor");
}

#[tokio::test]
async fn an_authoritative_day_leaves_other_days_alone() {
    let Some(server) = TestServer::start().await else { return };

    // Two days, each with its own task.
    server.post_day(&server.token, a_day()).await;
    let friday = json!({
        "date": "2026-08-15",
        "started_at": "2026-08-15T09:00:00-03:00",
        "tasks": [{"agent_task_id": 9, "recorded_at": "2026-08-15T17:00:00-03:00", "name": "Friday work", "completeness": 50}]
    });
    server.post_day(&server.token, friday).await;
    assert_eq!(server.count("tasks").await, 3);

    // The agent backfills Thursday with an authoritative, empty task list. This
    // is the case that makes the deletion date-scoped: a blunter "replace the
    // user's tasks" would take Friday's row with it.
    let mut empty_thursday = a_day();
    empty_thursday["tasks"] = json!([]);
    empty_thursday["tasks_are_complete"] = json!(true);

    let (status, body) = server.post_day(&server.token, empty_thursday).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted_tasks"], 2, "both of Thursday's tasks are gone");

    assert_eq!(server.count("tasks").await, 1, "Friday must be untouched");
    let survivor: i32 = server.scalar("SELECT agent_task_id FROM tasks").await;
    assert_eq!(survivor, 9, "and the survivor is Friday's task");
}

#[tokio::test]
async fn an_agent_that_does_not_claim_completeness_deletes_nothing() {
    let Some(server) = TestServer::start().await else { return };

    server.post_day(&server.token, a_day()).await;

    // An agent from before the flag existed: same day, fewer tasks, no claim.
    // Its silence must not be read as a deletion.
    let mut partial = a_day();
    partial["tasks"] = json!([{"agent_task_id": 1, "recorded_at": "2026-08-14T17:50:00-03:00", "name": "Write the ingest endpoint", "completeness": 90}]);

    let (status, body) = server.post_day(&server.token, partial).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted_tasks"], 0);

    assert_eq!(server.count("tasks").await, 2, "an older agent must not lose the employee's data");
}

/// A bare day for a date - what a backfilled day looks like at its simplest.
fn day_on(date: &str) -> Value {
    json!({
        "date": date,
        "started_at": format!("{date}T09:00:00-03:00"),
        "ended_at": format!("{date}T18:00:00-03:00")
    })
}

#[tokio::test]
async fn a_week_offline_arrives_in_one_request() {
    let Some(server) = TestServer::start().await else { return };

    let week: Vec<Value> = ["2026-08-10", "2026-08-11", "2026-08-12", "2026-08-13", "2026-08-14"]
        .iter()
        .map(|date| day_on(date))
        .collect();

    let (status, body) = server.post_batch(&server.token, json!(week)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["accepted"], 5);
    assert_eq!(body["rejected"], 0);
    assert_eq!(body["results"][0]["status"], "accepted");
    assert_eq!(body["results"][0]["date"], "2026-08-10");

    assert_eq!(server.count("workdays").await, 5, "every day of the backlog must land");
}

#[tokio::test]
async fn one_bad_day_does_not_sink_the_batch() {
    let Some(server) = TestServer::start().await else { return };

    // The middle day is impossible: it ends before it starts. The agent holding
    // it must still be able to deliver the rest, or it retries forever.
    let days = json!([
        day_on("2026-08-10"),
        {"date": "2026-08-11", "started_at": "2026-08-11T18:00:00-03:00", "ended_at": "2026-08-11T09:00:00-03:00"},
        day_on("2026-08-12"),
    ]);

    let (status, body) = server.post_batch(&server.token, days).await;
    assert_eq!(status, StatusCode::OK, "the request itself was processed: {body}");
    assert_eq!(body["accepted"], 2);
    assert_eq!(body["rejected"], 1);

    assert_eq!(body["results"][1]["status"], "rejected");
    assert_eq!(body["results"][1]["date"], "2026-08-11", "the agent must learn which day to drop");
    assert_eq!(
        body["results"][1]["error"], "ended_at is before started_at",
        "and why, so it does not retry what will never be accepted"
    );

    assert_eq!(server.count("workdays").await, 2, "the good days must be stored");
    let dates: Vec<String> = sqlx::query_scalar("SELECT to_char(date, 'YYYY-MM-DD') FROM workdays ORDER BY date")
        .fetch_all(&server.pool)
        .await
        .expect("failed to read the stored dates");
    assert_eq!(dates, vec!["2026-08-10", "2026-08-12"], "and the bad one must not be");
}

#[tokio::test]
async fn a_server_fault_stops_the_batch_instead_of_blaming_the_day() {
    let Some(server) = TestServer::start().await else { return };

    // A failure that is the server's, not the payload's - the same trick as the
    // single-day transaction test. Reporting it as "this day is rejected" would
    // tell the agent to drop a day it should have retried.
    server
        .execute("ALTER TABLE tasks ADD CONSTRAINT tasks_name_length CHECK (length(name) <= 40)")
        .await;

    let mut doomed = day_on("2026-08-11");
    doomed["tasks"] = json!([{"agent_task_id": 1, "recorded_at": "2026-08-11T17:00:00-03:00", "name": "x".repeat(60), "completeness": 50}]);

    let (status, body) = server
        .post_batch(&server.token, json!([day_on("2026-08-10"), doomed, day_on("2026-08-12")]))
        .await;

    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "a fault on our side must be reported as ours: {body}"
    );
    assert_eq!(body["error"], "internal server error", "and without leaking its cause");

    // The days before the fault stay written: each is its own transaction, and
    // the agent learns they landed by re-sending them - which is safe, because
    // the last upload wins.
    assert_eq!(server.count("workdays").await, 1, "the day before the fault is committed");
}

#[tokio::test]
async fn a_batch_is_bounded() {
    let Some(server) = TestServer::start().await else { return };

    let mut config = kasl_server::config::Config::defaults_for_database(String::new());
    config.max_batch_days = 2;

    let days = json!([day_on("2026-08-10"), day_on("2026-08-11"), day_on("2026-08-12")]);
    let (status, body) = server.post_batch_with_limits(&server.token, days, &config).await;

    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "an oversized batch must be refused: {body}");
    assert!(
        body["error"].as_str().unwrap_or_default().contains("at most 2 days"),
        "the limit belongs in the message, so the agent can split to fit: {body}"
    );
    assert_eq!(server.count("workdays").await, 0, "nothing from a refused batch may be stored");

    // Exactly at the limit is allowed - "at most" has to mean what it says, or
    // an agent splitting its backlog to fit gets refused anyway.
    let at_the_limit = json!([day_on("2026-08-10"), day_on("2026-08-11")]);
    let (status, body) = server.post_batch_with_limits(&server.token, at_the_limit, &config).await;
    assert_eq!(status, StatusCode::OK, "a batch of exactly the limit must be accepted: {body}");
    assert_eq!(server.count("workdays").await, 2);
}

#[tokio::test]
async fn an_oversized_body_is_refused_before_it_is_read() {
    let Some(server) = TestServer::start().await else { return };

    let mut config = kasl_server::config::Config::defaults_for_database(String::new());
    config.max_body_bytes = 512;

    // Well under the day limit, well over the byte limit: the two bounds have
    // to hold independently, or a batch of few enormous days slips through.
    let mut fat = day_on("2026-08-10");
    fat["tasks"] = json!([{"agent_task_id": 1, "recorded_at": "2026-08-10T17:00:00-03:00", "name": "x".repeat(2000), "completeness": 50}]);

    let (status, _) = server.post_batch_with_limits(&server.token, json!([fat]), &config).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "a body past the limit must not be buffered");
    assert_eq!(server.count("workdays").await, 0);
}

#[tokio::test]
async fn a_batch_needs_a_token_like_any_other_upload() {
    let Some(server) = TestServer::start().await else { return };

    let (status, _) = server.post_batch("not-a-real-token", json!([day_on("2026-08-10")])).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "the batch route must not be a way around the check");
    assert_eq!(server.count("workdays").await, 0);
}
