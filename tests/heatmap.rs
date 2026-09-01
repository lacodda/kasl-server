//! The team's month, against a live database.
//!
//! The cases split in two. Half are the permission - the same clause as every
//! other route answering for other people, asserted here rather than assumed,
//! because a second endpoint is a second chance for one copy of the rule to
//! widen quietly. The other half is what a cell says when there is nothing in
//! it: the whole value of a heatmap is that an empty square means "no data"
//! and not "no work", and only a test holds those apart.

mod support;

use axum::http::StatusCode;
use serde_json::{Value, json};
use support::TestServer;

/// A finished day of nine hours with half an hour of lunch in it.
fn day(date: &str) -> Value {
    json!({
        "date": date,
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
    })
}

/// The worked seconds of `day`: nine hours less the half-hour break.
const WORKED: i64 = 9 * 3600 - 1800;

/// An administrator, a manager with a department, an employee in it, and an
/// employee outside it - the same cast the dashboard's tests use.
struct Team {
    admin: Option<String>,
    manager: Option<String>,
    employee: Option<String>,
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

    Team { admin, manager, employee }
}

/// The names in a heatmap answer, in the order it listed them.
fn names(body: &Value) -> Vec<&str> {
    body["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row["display_name"].as_str().expect("a name"))
        .collect()
}

/// One person's row, by the name `provision` gave them.
fn row<'a>(body: &'a Value, name: &str) -> &'a Value {
    body["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|row| row["display_name"] == name)
        .unwrap_or_else(|| panic!("{name} should have a row: {body}"))
}

#[tokio::test]
async fn a_month_answers_a_cell_only_where_a_day_was_recorded() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    // Two days in the month asked for, one in the next. The gap between the
    // 3rd and the 10th is the assertion: it must be absent, not zero.
    for date in ["2026-03-03", "2026-03-10", "2026-04-01"] {
        let (status, body) = server.post_day("inside-token", day(date)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    let (status, body) = server.get_with_cookie("/api/v1/team/heatmap?month=2026-03", team.admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    assert_eq!(body["month"], "2026-03", "{body}");
    assert_eq!(body["from"], "2026-03-01", "{body}");
    assert_eq!(body["to"], "2026-03-31", "the calendar decides the end, not a 30-day window: {body}");

    let inside = row(&body, "inside");
    let days = inside["days"].as_array().expect("days");
    assert_eq!(days.len(), 2, "April is another month: {body}");
    assert_eq!(days[0]["date"], "2026-03-03");
    assert_eq!(days[1]["date"], "2026-03-10");
    // The point of the whole endpoint: the 4th to the 9th are not in the
    // answer at all. A zero there would say "worked nothing", which is a claim
    // about a person nobody made.
    assert_eq!(days[0]["worked_seconds"], WORKED, "{body}");
    assert_eq!(inside["worked_seconds"], 2 * WORKED, "{body}");
    assert_eq!(inside["busiest_seconds"], WORKED, "{body}");
    assert_eq!(body["busiest_seconds"], WORKED, "the grid's ceiling is the busiest day in it: {body}");
}

#[tokio::test]
async fn a_person_with_nothing_recorded_is_an_empty_row_not_a_missing_one() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    let (status, body) = server.get_with_cookie("/api/v1/team/heatmap?month=2026-03", team.admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Nobody uploaded anything. Everyone is still listed - the blank row is
    // exactly what a manager is scanning the grid for.
    let inside = row(&body, "inside");
    assert_eq!(inside["days"], json!([]), "{body}");
    assert_eq!(inside["worked_seconds"], 0, "{body}");
    assert_eq!(inside["busiest_seconds"], Value::Null, "no finished day means no busiest one: {body}");
    assert_eq!(body["busiest_seconds"], Value::Null, "an empty grid has no ceiling: {body}");
}

#[tokio::test]
async fn an_open_day_is_a_cell_with_no_hours() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    let today = chrono::Utc::now().date_naive();
    let month = today.format("%Y-%m").to_string();
    let mut open = day(&today.to_string());
    open.as_object_mut().expect("an object").remove("ended_at");
    let (status, body) = server.post_day("inside-token", open).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = server
        .get_with_cookie(&format!("/api/v1/team/heatmap?month={month}"), team.admin.as_deref())
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let inside = row(&body, "inside");
    let days = inside["days"].as_array().expect("days");
    assert_eq!(days.len(), 1, "{body}");
    assert_eq!(days[0]["open"], true, "{body}");
    // A day still running has no total, exactly as `/me/days` answers it. Zero
    // here would draw a full day as the palest square on the grid.
    assert_eq!(days[0]["worked_seconds"], Value::Null, "{body}");
    assert_eq!(inside["worked_seconds"], 0, "an open day adds nothing to the month: {body}");
    assert_eq!(inside["busiest_seconds"], Value::Null, "{body}");
}

#[tokio::test]
async fn hours_stay_honest_when_pauses_are_not_stored_one_by_one() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    // Narrow first: filtering happens on the way in, so this is the day as
    // `coarse` really stores it - no pause rows, only the day's own totals
    // (ADR 0011). The cell must read the surviving source, and read it once.
    let (status, _, body) = server
        .put_with_cookie("/api/v1/privacy", team.admin.as_deref(), json!({ "level": "coarse" }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = server.post_day("inside-token", day("2026-03-03")).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = server.get_with_cookie("/api/v1/team/heatmap?month=2026-03", team.admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Counting both sources would subtract the break twice and paint an
    // ordinary day as a light one - the grid quietly accusing someone.
    assert_eq!(row(&body, "inside")["days"][0]["worked_seconds"], WORKED, "{body}");
}

#[tokio::test]
async fn a_manager_sees_their_department_and_an_administrator_everyone() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    let (status, body) = server.get_with_cookie("/api/v1/team/heatmap?month=2026-03", team.manager.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let seen = names(&body);
    assert!(seen.contains(&"inside"), "their department: {body}");
    assert!(seen.contains(&"lead"), "a manager must find their own row: {body}");
    // The absences are the assertion. An unfiled employee belongs to the
    // administrator alone (ADR 0009), and this endpoint pastes in the same
    // clause rather than a second reading of it.
    assert!(!seen.contains(&"outside"), "an unfiled employee is not theirs to see: {body}");
    assert!(!seen.contains(&"boss@example.test"), "{body}");

    let (status, body) = server.get_with_cookie("/api/v1/team/heatmap?month=2026-03", team.admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let seen = names(&body);
    for name in ["inside", "outside", "lead", "boss@example.test"] {
        assert!(seen.contains(&name), "the admin should see {name}: {body}");
    }
}

#[tokio::test]
async fn an_employee_is_refused_and_so_is_an_agent_token() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    // Not an empty grid: an employee has no business learning the endpoint
    // answered at all.
    let (status, body) = server.get_with_cookie("/api/v1/team/heatmap?month=2026-03", team.employee.as_deref()).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    let (status, body) = server.get_with_cookie("/api/v1/team/heatmap?month=2026-03", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    // An agent token writes days and reads the manifest. It is not a person.
    let (status, body) = server.get_with_header("/api/v1/team/heatmap?month=2026-03", Some("Bearer inside-token")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
}

#[tokio::test]
async fn a_month_that_is_not_a_month_is_refused_rather_than_guessed() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    for bad in ["2026-13", "August", "2026-03-15", "2026"] {
        let (status, body) = server
            .get_with_cookie(&format!("/api/v1/team/heatmap?month={bad}"), team.admin.as_deref())
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "`{bad}` should be refused: {body}");
    }

    // Missing entirely is the same class of mistake, and must not default to
    // the current month: a caller who forgot the parameter would then get a
    // plausible answer to a question they did not ask.
    let (status, body) = server.get_with_cookie("/api/v1/team/heatmap", team.admin.as_deref()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
async fn a_deactivated_person_leaves_the_grid() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    let (status, body) = server.post_day("inside-token", day("2026-03-03")).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, users) = server.get_with_cookie("/api/v1/users", team.admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{users}");
    let inside_id = users
        .as_array()
        .expect("users")
        .iter()
        .find(|user| user["email"] == "inside@example.test")
        .expect("the account exists")["id"]
        .as_str()
        .expect("an id")
        .to_string();

    let (status, _, body) = server
        .patch_with_cookie(&format!("/api/v1/users/{inside_id}"), team.admin.as_deref(), json!({ "active": false }))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let (status, body) = server.get_with_cookie("/api/v1/team/heatmap?month=2026-03", team.admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!names(&body).contains(&"inside"), "a former employee is not on this month's grid: {body}");

    // Their days are still in the database - leaving the dashboard is not
    // erasure, and the dashboard's own test says the same about `/team/days`.
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM workdays")
        .fetch_one(&server.pool)
        .await
        .expect("the query should run");
    assert_eq!(kept, 1, "deactivating a person keeps their history");
}
