//! Signals and the weekly trend, against a live database.
//!
//! The arithmetic itself is unit-tested in the module, where a week is a
//! number rather than a row. What is only reachable here is everything
//! between the two: that the query groups days into the weeks the statistics
//! expect, that the permission clause applies to a second pair of routes, and
//! that a signal computed from real workdays says what it said in the unit
//! test.
//!
//! Days are uploaded relative to today, because the window is. A fixture with
//! fixed dates would pass in the week it was written and stop testing anything
//! the week after.

mod support;

use axum::http::StatusCode;
use chrono::{Datelike, Duration, NaiveDate, Utc};
use serde_json::{Value, json};
use support::TestServer;

/// The Monday of the current week on the server's own calendar.
fn this_monday() -> NaiveDate {
    let today = Utc::now().date_naive();
    today - Duration::days(i64::from(today.weekday().num_days_from_monday()))
}

/// The Monday `weeks_ago` complete weeks back - week 1 is the last complete
/// one, which is the most recent week any signal may look at.
fn monday_weeks_ago(weeks_ago: i64) -> NaiveDate {
    this_monday() - Duration::weeks(weeks_ago)
}

/// A finished day of `hours`, on `date`.
fn day(date: NaiveDate, hours: i64) -> Value {
    json!({
        "date": date.to_string(),
        "started_at": format!("{date}T09:00:00-03:00"),
        "ended_at": format!("{date}T{:02}:00:00-03:00", 9 + hours),
        "pauses": [],
        "tasks": []
    })
}

/// Uploads five weekdays of `hours` each, in the week `weeks_ago` back.
async fn upload_week(server: &TestServer, token: &str, weeks_ago: i64, hours: i64) {
    let monday = monday_weeks_ago(weeks_ago);
    for weekday in 0..5 {
        let date = monday + Duration::days(weekday);
        let (status, body) = server.post_day(token, day(date, hours)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
}

/// The cast: an administrator, a manager with a department, an employee in it,
/// and an employee outside it.
struct Team {
    admin: Option<String>,
    manager: Option<String>,
    employee: Option<String>,
    inside_id: String,
    outside_id: String,
}

async fn team(server: &TestServer) -> Team {
    server.add_admin("boss@example.test", "correct horse").await;
    let (_, admin, _) = server.login("boss@example.test", "correct horse").await;

    server.add_agent("inside@example.test", "inside-token").await;
    server.add_agent("outside@example.test", "outside-token").await;
    server.add_agent("lead@example.test", "lead-token").await;
    for email in ["inside@example.test", "outside@example.test", "lead@example.test"] {
        server.set_password(email, "correct horse").await;
    }

    let (status, users) = server.get_with_cookie("/api/v1/users", admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{users}");
    let id_of = |email: &str| -> String {
        users
            .as_array()
            .expect("a list of users")
            .iter()
            .find(|user| user["email"] == email)
            .expect("the account was just created")["id"]
            .as_str()
            .expect("an id")
            .to_string()
    };

    let lead_id = id_of("lead@example.test");
    let inside_id = id_of("inside@example.test");
    let outside_id = id_of("outside@example.test");

    let (status, _, body) = server
        .patch_with_cookie(&format!("/api/v1/users/{lead_id}"), admin.as_deref(), json!({ "role": "manager" }))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let (status, _, body) = server
        .post_with_cookie("/api/v1/departments", admin.as_deref(), json!({ "name": "Engineering", "manager_id": lead_id }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let department_id = body["id"].as_str().expect("the department should have an id").to_string();

    let (status, _, body) = server
        .put_with_cookie(
            &format!("/api/v1/users/{inside_id}/department"),
            admin.as_deref(),
            json!({ "department_id": department_id }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let (_, manager, _) = server.login("lead@example.test", "correct horse").await;
    let (_, employee, _) = server.login("inside@example.test", "correct horse").await;

    Team {
        admin,
        manager,
        employee,
        inside_id,
        outside_id,
    }
}

/// The signals about one person, by the name `provision` gave them.
fn signals_for<'a>(body: &'a Value, name: &str) -> Vec<&'a Value> {
    body["signals"]
        .as_array()
        .expect("signals")
        .iter()
        .filter(|signal| signal["display_name"] == name)
        .collect()
}

/// The kinds in an answer, in the order it listed them.
fn kinds(body: &Value) -> Vec<&str> {
    body["signals"]
        .as_array()
        .expect("signals")
        .iter()
        .map(|signal| signal["kind"].as_str().expect("a kind"))
        .collect()
}

#[tokio::test]
async fn falling_hours_are_reported_with_the_figures_behind_them() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    // Six weeks: three steady, then three falling. The last complete week is
    // week 1, and the run has to be found there rather than in the middle.
    for (weeks_ago, hours) in [(6, 8), (5, 8), (4, 8), (3, 7), (2, 6), (1, 5)] {
        upload_week(&server, "inside-token", weeks_ago, hours).await;
    }

    let (status, body) = server.get_with_cookie("/api/v1/team/signals", team.admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let found = signals_for(&body, "inside");
    let declining = found
        .iter()
        .find(|signal| signal["kind"] == "declining")
        .unwrap_or_else(|| panic!("the slide should be flagged: {body}"));

    assert_eq!(declining["weeks"], 3, "{body}");
    // The figures travel with the signal so the screen can say what happened
    // rather than that something is wrong: 40 h a week down to 25 h.
    assert_eq!(declining["from_seconds"], 5 * 8 * 3600, "{body}");
    assert_eq!(declining["to_seconds"], 5 * 5 * 3600, "{body}");
}

#[tokio::test]
async fn a_steady_person_is_counted_but_not_listed() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    for weeks_ago in 1..=6 {
        upload_week(&server, "inside-token", weeks_ago, 8).await;
    }

    let (status, body) = server.get_with_cookie("/api/v1/team/signals", team.admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    assert!(signals_for(&body, "inside").is_empty(), "a steady person is not news: {body}");
    // But they were examined. "Nothing found among five people" and "nothing
    // found because nobody was looked at" are different answers, and a screen
    // that cannot tell them apart shows the reassuring one. Five: the four
    // above plus the account `TestServer` provisions for itself.
    assert_eq!(body["people"], 5, "{body}");
}

#[tokio::test]
async fn a_gap_in_the_weeks_is_not_a_decline() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    // A fortnight off in the middle of an otherwise flat stretch. Reading the
    // absent weeks as zeroes would invent a crash and a recovery, and report
    // a holiday as a problem.
    for (weeks_ago, hours) in [(6, 8), (5, 8), (2, 8), (1, 8)] {
        upload_week(&server, "inside-token", weeks_ago, hours).await;
    }

    let (status, body) = server.get_with_cookie("/api/v1/team/signals", team.admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let found = signals_for(&body, "inside");
    assert!(
        !found.iter().any(|signal| signal["kind"] == "declining"),
        "a gap is missing data, not falling hours: {body}"
    );
}

#[tokio::test]
async fn an_agent_that_stopped_is_reported_as_silence() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    // Worked for four weeks, then nothing for the last three.
    for weeks_ago in 4..=7 {
        upload_week(&server, "inside-token", weeks_ago, 8).await;
    }

    let (status, body) = server.get_with_cookie("/api/v1/team/signals", team.admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let found = signals_for(&body, "inside");
    let quiet = found
        .iter()
        .find(|signal| signal["kind"] == "no_data")
        .unwrap_or_else(|| panic!("the silence should be flagged: {body}"));

    // Measured from the last real day, not from the edge of the window.
    let days = quiet["days_quiet"].as_i64().expect("a day count");
    assert!(days >= 17, "three weeks of silence, got {days}: {body}");
}

#[tokio::test]
async fn someone_who_never_reported_is_not_called_silent() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    // Nobody uploaded anything at all. The dashboard already says "never
    // reported" next to an agent count; "no data for 84 days" about somebody
    // who never had any is arithmetic dressed up as an observation.
    let (status, body) = server.get_with_cookie("/api/v1/team/signals", team.admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    assert_eq!(body["signals"], json!([]), "{body}");
    assert_eq!(body["people"], 5, "everyone was still examined: {body}");
}

#[tokio::test]
async fn an_unusual_week_names_the_median_it_is_unusual_against() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    // Four ordinary weeks and then a very short one. Not a decline - it is one
    // week, not a run - but it is unlike this person's own usual.
    for (weeks_ago, hours) in [(5, 8), (4, 8), (3, 8), (2, 8), (1, 3)] {
        upload_week(&server, "inside-token", weeks_ago, hours).await;
    }

    let (status, body) = server.get_with_cookie("/api/v1/team/signals", team.admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let found = signals_for(&body, "inside");
    let unusual = found
        .iter()
        .find(|signal| signal["kind"] == "unusual_week")
        .unwrap_or_else(|| panic!("a week at a third of the usual is unusual: {body}"));

    assert_eq!(unusual["to_seconds"], 5 * 3 * 3600, "{body}");
    // The median travels with it: a deviation without the thing deviated from
    // is a percentage of nothing.
    assert_eq!(unusual["median_seconds"], 5 * 8 * 3600, "{body}");
}

#[tokio::test]
async fn the_current_week_never_enters_the_arithmetic() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    // Four full weeks behind, and today's part-week deliberately tiny. A
    // partial week looks like a collapse to arithmetic; including it would
    // fire on the whole team every Monday morning.
    for weeks_ago in 1..=4 {
        upload_week(&server, "inside-token", weeks_ago, 8).await;
    }
    let today = Utc::now().date_naive();
    let (status, body) = server.post_day("inside-token", day(today, 1)).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = server.get_with_cookie("/api/v1/team/signals", team.admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    assert!(signals_for(&body, "inside").is_empty(), "today's single hour is not a week: {body}");
    // And the window says so: it ends before this week began.
    let to: NaiveDate = body["to"].as_str().expect("a date").parse().expect("a date");
    assert!(to < this_monday(), "the window must stop at the last complete week: {body}");
}

#[tokio::test]
async fn the_trend_draws_every_week_including_the_empty_ones() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    for (weeks_ago, hours) in [(4, 8), (2, 8), (1, 8)] {
        upload_week(&server, "inside-token", weeks_ago, hours).await;
    }

    let (status, body) = server
        .get_with_cookie(&format!("/api/v1/users/{}/trend", team.inside_id), team.manager.as_deref())
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let weeks = body["weeks"].as_array().expect("weeks");
    assert_eq!(weeks.len(), 12, "twelve complete weeks, however few have data: {body}");

    // The gap is drawn as a gap. Dropping the empty week would close the hole
    // up and make an absence look like continuity.
    let third_from_last = &weeks[weeks.len() - 3];
    assert_eq!(third_from_last["worked_seconds"], 0, "{body}");
    assert_eq!(third_from_last["days_recorded"], 0, "zero days says the silence is real: {body}");
    assert_eq!(weeks[weeks.len() - 1]["worked_seconds"], 5 * 8 * 3600, "{body}");
}

#[tokio::test]
async fn the_trend_carries_the_signals_about_that_person() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    for (weeks_ago, hours) in [(6, 8), (5, 8), (4, 8), (3, 7), (2, 6), (1, 5)] {
        upload_week(&server, "inside-token", weeks_ago, hours).await;
    }

    let (status, body) = server
        .get_with_cookie(&format!("/api/v1/users/{}/trend", team.inside_id), team.manager.as_deref())
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // The drill-down is where a signal leads, so the reason for the trip has
    // to be on the page it arrives at.
    let kinds: Vec<&str> = body["signals"]
        .as_array()
        .expect("signals")
        .iter()
        .map(|signal| signal["kind"].as_str().expect("a kind"))
        .collect();
    assert!(kinds.contains(&"declining"), "{body}");
    assert_ne!(body["median_seconds"], Value::Null, "{body}");
}

#[tokio::test]
async fn a_manager_sees_their_department_and_an_administrator_everyone() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    // Both people slide, but only one is the manager's to know about.
    for token in ["inside-token", "outside-token"] {
        for (weeks_ago, hours) in [(4, 8), (3, 7), (2, 6), (1, 5)] {
            upload_week(&server, token, weeks_ago, hours).await;
        }
    }

    let (status, body) = server.get_with_cookie("/api/v1/team/signals", team.manager.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!signals_for(&body, "inside").is_empty(), "their own department: {body}");
    // The absence is the assertion: an unfiled employee belongs to the
    // administrator alone (ADR 0009), and this is a second pair of routes
    // where one copy of the rule could quietly widen.
    assert!(signals_for(&body, "outside").is_empty(), "not theirs to see: {body}");

    let (status, body) = server.get_with_cookie("/api/v1/team/signals", team.admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!signals_for(&body, "outside").is_empty(), "the admin sees everyone: {body}");
}

#[tokio::test]
async fn someone_elses_employee_is_indistinguishable_from_a_stranger() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    // A real id the manager may not see, and an id belonging to nobody. Both
    // answer 404: telling them apart would turn the route into a way to map
    // the company one uuid at a time.
    let (real, body) = server
        .get_with_cookie(&format!("/api/v1/users/{}/trend", team.outside_id), team.manager.as_deref())
        .await;
    assert_eq!(real, StatusCode::NOT_FOUND, "{body}");

    let (invented, body) = server
        .get_with_cookie("/api/v1/users/00000000-0000-0000-0000-000000000000/trend", team.manager.as_deref())
        .await;
    assert_eq!(invented, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test]
async fn an_employee_is_refused_and_so_is_an_agent_token() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    // Not an empty list: an employee has no business learning that the server
    // computes signals about their colleagues at all.
    let (status, body) = server.get_with_cookie("/api/v1/team/signals", team.employee.as_deref()).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // Nor about themselves through this route - the trend is a manager's view,
    // and what an employee may read about themselves is `/me/days`.
    let (status, body) = server
        .get_with_cookie(&format!("/api/v1/users/{}/trend", team.inside_id), team.employee.as_deref())
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    let (status, body) = server.get_with_cookie("/api/v1/team/signals", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    // An agent token writes days and reads the manifest. It is not a person.
    let (status, body) = server.get_with_header("/api/v1/team/signals", Some("Bearer inside-token")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
}

#[tokio::test]
async fn silence_outranks_a_slide_in_the_listing() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    // One person sliding, another gone quiet. A manager reads top-down, and an
    // agent that stopped reporting means the other numbers about that person
    // are not to be trusted either.
    for (weeks_ago, hours) in [(6, 8), (5, 8), (4, 8), (3, 7), (2, 6), (1, 5)] {
        upload_week(&server, "inside-token", weeks_ago, hours).await;
    }
    for weeks_ago in 4..=7 {
        upload_week(&server, "outside-token", weeks_ago, 8).await;
    }

    let (status, body) = server.get_with_cookie("/api/v1/team/signals", team.admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let listed = kinds(&body);
    assert_eq!(listed.first(), Some(&"no_data"), "silence comes first: {body}");
    assert!(listed.contains(&"declining"), "{body}");
}

#[tokio::test]
async fn hours_stay_honest_when_pauses_are_not_stored_one_by_one() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    // Narrow first: under `coarse` the day carries its own totals and no pause
    // rows (ADR 0011). The weekly sums must read the surviving source, and
    // read it once - counting both would shrink every week by the same break
    // twice and manufacture a decline out of a policy change.
    let (status, _, body) = server
        .put_with_cookie("/api/v1/privacy", team.admin.as_deref(), json!({ "level": "coarse" }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let monday = monday_weeks_ago(1);
    let date = monday + Duration::days(1);
    let with_pause = json!({
        "date": date.to_string(),
        "started_at": format!("{date}T09:00:00-03:00"),
        "ended_at": format!("{date}T18:00:00-03:00"),
        "pauses": [{
            "started_at": format!("{date}T12:00:00-03:00"),
            "ended_at": format!("{date}T12:30:00-03:00"),
            "duration_seconds": 1800,
            "manual": true,
            "reason": "lunch"
        }],
        "tasks": []
    });
    let (status, body) = server.post_day("inside-token", with_pause).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = server
        .get_with_cookie(&format!("/api/v1/users/{}/trend", team.inside_id), team.admin.as_deref())
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let weeks = body["weeks"].as_array().expect("weeks");
    let last = &weeks[weeks.len() - 1];
    assert_eq!(last["worked_seconds"], 9 * 3600 - 1800, "the break is subtracted once: {body}");
}
