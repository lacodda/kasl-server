//! Reading your own days back, against a live database.
//!
//! The claim under test is that the two ends of the server agree: what an
//! agent uploaded by bearer token is what the person sees after signing in
//! with a cookie. Every test here therefore writes through the real ingest
//! path rather than inserting rows, because a query that reassembles days
//! correctly from fixtures it wrote itself proves nothing about the schema the
//! agent actually fills.

mod support;

use axum::http::StatusCode;
use serde_json::{Value, json};
use support::TestServer;

/// The day the tests read back: two pauses, one of them a manual break with a
/// reason, and two tasks.
///
/// Task ids come from `date`, because the agent's `agent_task_id` is unique per
/// user and not per day: two days sharing one would be the same task moved, not
/// two tasks, and the server would rightly carry it to the later date.
fn day(date: &str) -> Value {
    // A distinct base per date, in the range an agent's own row ids live in.
    let base: i32 = date.replace('-', "")[4..].parse::<i32>().expect("a test date") * 10;
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
                "reason": "lunch"
            },
            {
                "started_at": format!("{date}T15:00:00-03:00"),
                "ended_at": format!("{date}T15:10:00-03:00"),
                "duration_seconds": 600,
                "manual": false
            }
        ],
        "tasks": [
            { "agent_task_id": base + 1, "recorded_at": format!("{date}T17:00:00-03:00"), "name": "Ship the release", "comment": "went fine", "completeness": 100 },
            { "agent_task_id": base + 2, "recorded_at": format!("{date}T17:30:00-03:00"), "name": "Review the plan", "completeness": 40 }
        ]
    })
}

/// Signs the seeded employee in and returns their cookie.
///
/// The agent seeded by `TestServer::start` and this login are the same person:
/// provisioning creates the user, and giving them a password does not create a
/// second one. That is exactly the join these tests exist to check.
async fn sign_in(server: &TestServer) -> Option<String> {
    server.set_password("employee@example.test", "correct horse").await;
    let (status, cookie, body) = server.login("employee@example.test", "correct horse").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    cookie
}

#[tokio::test]
async fn a_day_comes_back_the_way_the_agent_sent_it() {
    let Some(server) = TestServer::start().await else { return };

    let (status, body) = server.post_day(&server.token, day("2026-08-24")).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let cookie = sign_in(&server).await;
    let (status, body) = server.get_with_cookie("/api/v1/me/days?from=2026-08-24&to=2026-08-24", cookie.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let days = body["days"].as_array().expect("days should be a list");
    assert_eq!(days.len(), 1, "one day was uploaded: {body}");
    let day = &days[0];

    assert_eq!(day["date"], "2026-08-24");
    assert_eq!(day["pauses"].as_array().unwrap().len(), 2);
    assert_eq!(day["tasks"].as_array().unwrap().len(), 2);
    // 09:00 to 18:00 is nine hours; 1800 + 600 seconds of it were paused.
    assert_eq!(day["worked_seconds"], 9 * 3600 - 2400);
    assert_eq!(day["paused_count"], 2);
    assert_eq!(day["paused_seconds"], 2400);

    // The manual break keeps both the flag and the words typed into it: under
    // the default level nothing is withheld, and the screen says so.
    let lunch = day["pauses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|pause| pause["manual"] == true)
        .expect("the manual pause should come back");
    assert_eq!(lunch["reason"], "lunch");
    assert_eq!(body["privacy_level"], "full");
    assert_eq!(body["not_stored"], json!([]));
}

#[tokio::test]
async fn only_the_days_in_the_range_come_back_and_in_order() {
    let Some(server) = TestServer::start().await else { return };

    // Uploaded out of order, so passing this cannot be an accident of
    // insertion order - the endpoint has to be the thing that sorts.
    for date in ["2026-08-26", "2026-08-24", "2026-08-25", "2026-08-27"] {
        let (status, body) = server.post_day(&server.token, day(date)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    let cookie = sign_in(&server).await;
    let (status, body) = server.get_with_cookie("/api/v1/me/days?from=2026-08-25&to=2026-08-26", cookie.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let dates: Vec<&str> = body["days"].as_array().unwrap().iter().map(|day| day["date"].as_str().unwrap()).collect();
    // Both ends inclusive, neighbours excluded, ascending.
    assert_eq!(dates, vec!["2026-08-25", "2026-08-26"], "{body}");
}

#[tokio::test]
async fn a_days_pauses_and_tasks_are_its_own() {
    let Some(server) = TestServer::start().await else { return };

    for date in ["2026-08-24", "2026-08-25"] {
        let (status, body) = server.post_day(&server.token, day(date)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    let cookie = sign_in(&server).await;
    let (status, body) = server.get_with_cookie("/api/v1/me/days?from=2026-08-24&to=2026-08-25", cookie.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // The grouping is where a wrong join shows up: two days each with two
    // pauses and two tasks would come back as four and four apiece, and the
    // hours would double with them.
    for day in body["days"].as_array().unwrap() {
        assert_eq!(
            day["pauses"].as_array().unwrap().len(),
            2,
            "day {} got someone else's pauses: {body}",
            day["date"]
        );
        assert_eq!(
            day["tasks"].as_array().unwrap().len(),
            2,
            "day {} got someone else's tasks: {body}",
            day["date"]
        );
        assert_eq!(day["paused_seconds"], 2400, "{body}");
    }
}

#[tokio::test]
async fn one_persons_days_are_not_anothers() {
    let Some(server) = TestServer::start().await else { return };

    // Two employees, each with an agent, each uploading the same date.
    let other = server.add_agent("other@example.test", "other-token").await;
    let (status, body) = server.post_day(&server.token, day("2026-08-24")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = server.post_day(&other, day("2026-08-24")).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let cookie = sign_in(&server).await;
    let (status, body) = server.get_with_cookie("/api/v1/me/days?from=2026-08-24&to=2026-08-24", cookie.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // One day, not two: `/me` is the signed-in person's own history, and a
    // missing `user_id` filter would show a colleague's day as theirs.
    let days = body["days"].as_array().unwrap();
    assert_eq!(days.len(), 1, "{body}");
    assert_eq!(days[0]["tasks"].as_array().unwrap().len(), 2, "the colleague's tasks must not join in: {body}");
}

#[tokio::test]
async fn an_open_day_reports_no_total() {
    let Some(server) = TestServer::start().await else { return };

    let mut today = day("2026-08-27");
    today.as_object_mut().unwrap().remove("ended_at");
    let (status, body) = server.post_day(&server.token, today).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let cookie = sign_in(&server).await;
    let (status, body) = server.get_with_cookie("/api/v1/me/days?from=2026-08-27&to=2026-08-27", cookie.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let day = &body["days"][0];
    assert_eq!(day["ended_at"], Value::Null);
    // A day still running has no total. Reporting the hours so far as the
    // day's figure would show every working afternoon as a short day.
    assert_eq!(day["worked_seconds"], Value::Null, "{body}");
    // What is known is still answered: the pauses happened either way.
    assert_eq!(day["paused_seconds"], 2400, "{body}");
}

#[tokio::test]
async fn a_narrowed_policy_says_what_it_withheld() {
    let Some(server) = TestServer::start().await else { return };

    // Narrow first, then upload: filtering happens on the way in, so this is
    // the day as `coarse` actually stores it (ADR 0011).
    server.add_admin("boss@example.test", "correct horse").await;
    let (_, admin_cookie, _) = server.login("boss@example.test", "correct horse").await;
    let (status, _, body) = server
        .put_with_cookie("/api/v1/privacy", admin_cookie.as_deref(), json!({ "level": "coarse" }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = server.post_day(&server.token, day("2026-08-24")).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let cookie = sign_in(&server).await;
    let (status, body) = server.get_with_cookie("/api/v1/me/days?from=2026-08-24&to=2026-08-24", cookie.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let day = &body["days"][0];
    assert_eq!(day["pauses"], json!([]));
    assert_eq!(day["tasks"], json!([]));
    // The empty lists above are the whole reason this field exists: without it
    // the screen cannot tell "no pauses were stored" from "you took none", and
    // would draw an unbroken nine-hour day.
    assert_eq!(body["not_stored"], json!(["pauses", "tasks", "free_text"]), "{body}");
    assert_eq!(body["privacy_level"], "coarse");
    // And the hours stay honest: the totals the day carries instead of rows.
    assert_eq!(day["paused_count"], 2, "{body}");
    assert_eq!(day["paused_seconds"], 2400, "{body}");
    assert_eq!(day["worked_seconds"], 9 * 3600 - 2400, "{body}");
}

#[tokio::test]
async fn signing_in_is_required() {
    let Some(server) = TestServer::start().await else { return };

    let (status, body) = server.get_with_cookie("/api/v1/me/days?from=2026-08-24&to=2026-08-24", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    // An agent token is not a person: it can write days and read the manifest,
    // and that is deliberately the whole list. `/me` answers a session.
    let (status, body) = server
        .get_with_header("/api/v1/me/days?from=2026-08-24&to=2026-08-24", Some(&format!("Bearer {}", server.token)))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
}

#[tokio::test]
async fn a_malformed_range_is_refused_before_the_database() {
    let Some(server) = TestServer::start().await else { return };
    let cookie = sign_in(&server).await;

    // Missing parameters, a date that is not one, and a range too wide: all
    // client mistakes, all answered as such rather than as an empty result.
    for query in ["", "?from=2026-08-24", "?from=2026-08-24&to=not-a-date", "?from=2026-01-01&to=2027-06-01"] {
        let (status, body) = server.get_with_cookie(&format!("/api/v1/me/days{query}"), cookie.as_deref()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "`{query}` should be refused: {body}");
    }
}
