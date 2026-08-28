//! The manager's view of other people, against a live database.
//!
//! Almost every test here is about the permission rather than the arithmetic:
//! this is the first place the server answers for someone other than the
//! caller, and a visibility bug is the one kind of defect its victim cannot
//! see. So the cases are drawn from who is asking - an employee, a manager with
//! a department, a manager without one, an administrator - and each asserts
//! what is *absent* as firmly as what is present.

mod support;

use axum::http::StatusCode;
use serde_json::{Value, json};
use support::TestServer;

/// A day with one pause and one task, for whichever agent uploads it.
fn day(date: &str, agent_task_id: i64) -> Value {
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
        "tasks": [{
            "agent_task_id": agent_task_id,
            "recorded_at": format!("{date}T17:00:00-03:00"),
            "name": "Ship the release",
            "completeness": 100
        }]
    })
}

/// The cast every test needs: an administrator, a manager with a department,
/// an employee in it, and an employee outside it.
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

    // Three people besides the admin. `provision` creates the accounts; giving
    // them passwords does not create second ones.
    server.add_agent("inside@example.test", "inside-token").await;
    server.add_agent("outside@example.test", "outside-token").await;
    server.add_agent("lead@example.test", "lead-token").await;
    for email in ["inside@example.test", "outside@example.test", "lead@example.test"] {
        server.set_password(email, "correct horse").await;
    }

    let ids = user_ids(server, admin.as_deref()).await;
    let lead_id = ids["lead@example.test"].clone();
    let inside_id = ids["inside@example.test"].clone();
    let outside_id = ids["outside@example.test"].clone();

    // The lead becomes a manager and gets a department with one member.
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

/// Every account's id, keyed by email, as the admin sees them.
async fn user_ids(server: &TestServer, admin: Option<&str>) -> std::collections::HashMap<String, String> {
    let (status, body) = server.get_with_cookie("/api/v1/users", admin).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body.as_array()
        .expect("a list of users")
        .iter()
        .map(|user| {
            (
                user["email"].as_str().expect("an email").to_string(),
                user["id"].as_str().expect("an id").to_string(),
            )
        })
        .collect()
}

/// The emails in a `/team/days` answer.
fn emails(body: &Value) -> Vec<&str> {
    body["members"]
        .as_array()
        .expect("members")
        .iter()
        .map(|m| m["email"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn a_manager_sees_their_department_and_themselves_and_nobody_else() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    let (status, body) = server
        .get_with_cookie("/api/v1/team/days?from=2026-08-24&to=2026-08-30", team.manager.as_deref())
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let seen = emails(&body);
    assert!(seen.contains(&"inside@example.test"), "their department: {body}");
    assert!(seen.contains(&"lead@example.test"), "a manager must find their own row: {body}");
    // The two absences are the point. An unfiled employee is the admin's alone
    // (ADR 0009), and the administrator is in no department of theirs.
    assert!(!seen.contains(&"outside@example.test"), "an unfiled employee is not theirs to see: {body}");
    assert!(!seen.contains(&"boss@example.test"), "{body}");
}

#[tokio::test]
async fn an_administrator_sees_everyone() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    let (status, body) = server
        .get_with_cookie("/api/v1/team/days?from=2026-08-24&to=2026-08-30", team.admin.as_deref())
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let seen = emails(&body);
    for email in ["inside@example.test", "outside@example.test", "lead@example.test", "boss@example.test"] {
        assert!(seen.contains(&email), "the admin should see {email}: {body}");
    }
}

#[tokio::test]
async fn an_employee_is_refused_the_team_entirely() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    // Not an empty list: an employee has no business knowing the endpoint
    // answered at all.
    let (status, body) = server
        .get_with_cookie("/api/v1/team/days?from=2026-08-24&to=2026-08-30", team.employee.as_deref())
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    let (status, body) = server
        .get_with_cookie(
            &format!("/api/v1/users/{}/days?from=2026-08-24&to=2026-08-30", team.inside_id),
            team.employee.as_deref(),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
}

#[tokio::test]
async fn signing_in_is_required() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    let (status, body) = server.get_with_cookie("/api/v1/team/days?from=2026-08-24&to=2026-08-30", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    // An agent token writes days and reads the manifest. It is not a person and
    // cannot read the team.
    let (status, body) = server
        .get_with_header(
            &format!("/api/v1/users/{}/days?from=2026-08-24&to=2026-08-30", team.inside_id),
            Some("Bearer inside-token"),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
}

#[tokio::test]
async fn someone_elses_employee_is_indistinguishable_from_a_stranger() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    // A real id the manager may not see, and an id belonging to nobody. Both
    // answer 404: telling them apart would turn this route into a way to map
    // the company one uuid at a time.
    let (real, body) = server
        .get_with_cookie(
            &format!("/api/v1/users/{}/days?from=2026-08-24&to=2026-08-30", team.outside_id),
            team.manager.as_deref(),
        )
        .await;
    assert_eq!(real, StatusCode::NOT_FOUND, "{body}");

    let (invented, body) = server
        .get_with_cookie(
            "/api/v1/users/00000000-0000-0000-0000-000000000000/days?from=2026-08-24&to=2026-08-30",
            team.manager.as_deref(),
        )
        .await;
    assert_eq!(invented, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test]
async fn the_drill_down_answers_the_same_shape_as_the_personal_page() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    let (status, body) = server.post_day("inside-token", day("2026-08-25", 1)).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, manager_view) = server
        .get_with_cookie(
            &format!("/api/v1/users/{}/days?from=2026-08-25&to=2026-08-25", team.inside_id),
            team.manager.as_deref(),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{manager_view}");

    let (status, own_view) = server
        .get_with_cookie("/api/v1/me/days?from=2026-08-25&to=2026-08-25", team.employee.as_deref())
        .await;
    assert_eq!(status, StatusCode::OK, "{own_view}");

    // Byte for byte the same answer. The drill-down is the personal screen
    // pointed at someone else, and two shapes would mean two renderers.
    assert_eq!(manager_view, own_view, "the manager's drill-down and the employee's own page must agree");
    assert_eq!(manager_view["days"][0]["worked_seconds"], 9 * 3600 - 1800);
}

#[tokio::test]
async fn hours_add_up_across_the_range_and_stop_at_its_edges() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    // Three days, one of them outside the range asked for.
    for (index, date) in ["2026-08-25", "2026-08-26", "2026-09-01"].iter().enumerate() {
        let (status, body) = server.post_day("inside-token", day(date, index as i64 + 1)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    let (status, body) = server
        .get_with_cookie("/api/v1/team/days?from=2026-08-24&to=2026-08-30", team.manager.as_deref())
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let inside = body["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["email"] == "inside@example.test")
        .expect("the department member should be listed");

    assert_eq!(inside["days_recorded"], 2, "September is outside the range: {body}");
    assert_eq!(inside["worked_seconds"], 2 * (9 * 3600 - 1800), "{body}");
    assert_eq!(inside["paused_seconds"], 2 * 1800, "{body}");
    assert_eq!(inside["last_day"], "2026-08-26", "{body}");

    // The summary reads pause rows *or* the day's totals, and picking one is
    // only safe because ingest never writes both: totals are stored exactly
    // when rows are not (ADR 0011). Checked here, where the days above do have
    // pause rows, so a change that started filling both in would fail. It is
    // asserted rather than assumed because the consequence is silent - the
    // dashboard would count each break twice and shrink the working day.
    let both: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM workdays w
         WHERE w.paused_seconds IS NOT NULL AND EXISTS (SELECT 1 FROM pauses p WHERE p.workday_id = w.id)",
    )
    .fetch_one(&server.pool)
    .await
    .expect("the invariant query should run");
    assert_eq!(both, 0, "a day carries pause rows or their totals, never both");
}

#[tokio::test]
async fn hours_stay_honest_when_pauses_are_not_stored_one_by_one() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    // Narrow first: filtering happens on the way in, so this is the day as
    // `coarse` actually stores it - no pause rows, only the day's own totals
    // (ADR 0011).
    let (status, _, body) = server
        .put_with_cookie("/api/v1/privacy", team.admin.as_deref(), json!({ "level": "coarse" }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = server.post_day("inside-token", day("2026-08-25", 1)).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = server
        .get_with_cookie("/api/v1/team/days?from=2026-08-24&to=2026-08-30", team.manager.as_deref())
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let inside = body["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["email"] == "inside@example.test")
        .unwrap();

    // The whole point of the two sources: with no pause rows the day's own
    // totals answer instead, and they answer once. Counting both would inflate
    // the break and shrink the working day - a dashboard that quietly accuses
    // someone of working less than they did.
    assert_eq!(inside["paused_seconds"], 1800, "{body}");
    assert_eq!(inside["worked_seconds"], 9 * 3600 - 1800, "{body}");
    assert_eq!(body["not_stored"], json!(["pauses", "tasks", "free_text"]), "{body}");
}

#[tokio::test]
async fn a_person_with_nothing_recorded_is_still_listed() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    let (status, body) = server
        .get_with_cookie("/api/v1/team/days?from=2026-08-24&to=2026-08-30", team.manager.as_deref())
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let inside = body["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["email"] == "inside@example.test")
        .unwrap();

    // Nobody uploaded anything in this test. The row exists anyway: an employee
    // whose agent never reported is exactly who a manager needs to notice, and
    // dropping them would hide the case the dashboard is for.
    assert_eq!(inside["days_recorded"], 0, "{body}");
    assert_eq!(inside["worked_seconds"], 0);
    assert_eq!(inside["last_day"], Value::Null, "no data is not a date: {body}");
    assert_eq!(inside["day_open"], false);
    // What explains the silence: they do have an agent, it just has not written.
    assert_eq!(inside["agents"], 1, "{body}");
    assert_eq!(inside["last_seen_at"], Value::Null, "{body}");
}

#[tokio::test]
async fn an_open_day_today_is_reported_as_open() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    // "Today" on the server, because that is what the query compares against.
    let today = chrono::Utc::now().date_naive().to_string();
    let mut open = day(&today, 7);
    open.as_object_mut().unwrap().remove("ended_at");
    let (status, body) = server.post_day("inside-token", open).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = server
        .get_with_cookie(&format!("/api/v1/team/days?from={today}&to={today}"), team.manager.as_deref())
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let inside = body["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["email"] == "inside@example.test")
        .unwrap();
    assert_eq!(inside["day_open"], true, "{body}");
    assert_eq!(inside["days_recorded"], 1);
    // An unfinished day contributes no hours, exactly as on the personal page.
    assert_eq!(inside["worked_seconds"], 0, "an open day has no total to add: {body}");
    // But the agent has been heard from, which is the honest half of "who is
    // working now".
    assert_ne!(inside["last_seen_at"], Value::Null, "{body}");
}

#[tokio::test]
async fn a_deactivated_person_leaves_the_dashboard_but_keeps_their_history() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    let (status, body) = server.post_day("inside-token", day("2026-08-25", 1)).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, _, body) = server
        .patch_with_cookie(&format!("/api/v1/users/{}", team.inside_id), team.admin.as_deref(), json!({ "active": false }))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let (status, body) = server
        .get_with_cookie("/api/v1/team/days?from=2026-08-24&to=2026-08-30", team.manager.as_deref())
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        !emails(&body).contains(&"inside@example.test"),
        "a deactivated person is not on today's dashboard: {body}"
    );

    // Their days are still there for whoever needs to look: deactivation ends
    // access, it does not erase a record (the schema's own promise).
    let (status, body) = server
        .get_with_cookie(
            &format!("/api/v1/users/{}/days?from=2026-08-24&to=2026-08-30", team.inside_id),
            team.admin.as_deref(),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["days"].as_array().unwrap().len(), 1, "{body}");
}

#[tokio::test]
async fn a_malformed_range_is_refused() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    for query in ["", "?from=2026-08-24", "?from=2026-01-01&to=2027-06-01"] {
        let (status, body) = server.get_with_cookie(&format!("/api/v1/team/days{query}"), team.manager.as_deref()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "`{query}` should be refused: {body}");
    }
}
