//! The BI API and its read tokens, against a live database (ADR 0025).
//!
//! Four kinds of claim. The columns are a contract: what was published stays,
//! in its place, and the reference page lists exactly what the server sends. A
//! token reads with its owner's eyes - the team table's cast of an
//! administrator, a manager with a department of one and an employee outside
//! it - and stops reading when its owner may not. The rows are the server's
//! own: a day is the timesheet's cell and a summary row is the team table's.
//! And the JSON and the CSV are one table, value for value.

mod support;

use std::collections::HashMap;

use axum::http::{StatusCode, header};
use serde_json::{Value, json};
use support::TestServer;

// The published columns ---------------------------------------------------------
//
// What v1 promised, copied here rather than read from the server: a column
// renamed or dropped in the code must fail a test, not quietly update the
// promise. Growing a dataset means appending to the server's list; these stay
// a prefix of it for as long as the path says v1.

const PUBLISHED_PEOPLE: &[&str] = &[
    "user_id",
    "display_name",
    "email",
    "role",
    "active",
    "department_id",
    "department",
    "work_rate",
    "created_at",
];
const PUBLISHED_DEPARTMENTS: &[&str] = &["department_id", "department", "manager_id", "manager"];
const PUBLISHED_DAYS: &[&str] = &[
    "date",
    "user_id",
    "display_name",
    "email",
    "department_id",
    "department",
    "state",
    "kind",
    "started_at",
    "ended_at",
    "worked_seconds",
    "paused_seconds",
    "norm_seconds",
    "filed",
    "report",
    "calendar",
    "weekend",
];
const PUBLISHED_SUMMARY: &[&str] = &[
    "user_id",
    "display_name",
    "email",
    "department_id",
    "department",
    "active",
    "work_rate",
    "days_worked",
    "days_away",
    "worked_seconds",
    "paused_seconds",
    "norm_seconds",
    "due_seconds",
    "last_day",
];

fn datasets() -> [(&'static str, &'static [&'static str], &'static [&'static str]); 4] {
    use kasl_server::bi;
    [
        ("people", PUBLISHED_PEOPLE, bi::PEOPLE),
        ("departments", PUBLISHED_DEPARTMENTS, bi::DEPARTMENTS),
        ("days", PUBLISHED_DAYS, bi::DAYS),
        ("summary", PUBLISHED_SUMMARY, bi::SUMMARY),
    ]
}

#[test]
fn what_v1_published_is_still_there_in_its_place() {
    for (name, published, served) in datasets() {
        assert!(
            served.starts_with(published),
            "`{name}` must keep its published columns, in order, and only grow at the end:\n published {published:?}\n served    {served:?}"
        );
    }
}

#[test]
fn the_reference_lists_every_column_in_order() {
    // Each dataset's section on the page has a table whose first column names
    // the fields in backticks. Read from the table rows only, so prose that
    // mentions a column elsewhere neither satisfies nor breaks the check.
    let page = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/src/content/docs/reference/the-bi-api.md"))
        .expect("the BI reference page should exist");

    for (name, _, served) in datasets() {
        let heading = format!("### `{name}`");
        let start = page.find(&heading).unwrap_or_else(|| panic!("the page has no `{heading}` section"));
        let section = &page[start + heading.len()..];
        let section = &section[..section.find("\n#").unwrap_or(section.len())];
        let listed: Vec<&str> = section
            .lines()
            .filter_map(|line| line.strip_prefix("| `"))
            .filter_map(|rest| rest.split_once('`').map(|(column, _)| column))
            .collect();
        assert_eq!(listed, served.to_vec(), "the `{name}` table on the reference page");
    }
}

// The cast --------------------------------------------------------------------------

/// A day with a half-hour lunch: nine hours on the clock, eight and a half
/// worked.
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

/// A day the person was on leave.
fn leave(date: &str) -> Value {
    json!({
        "date": date,
        "kind": "vacation",
        "started_at": format!("{date}T09:00:00-03:00"),
        "ended_at": format!("{date}T09:00:00-03:00"),
        "pauses": [],
        "tasks": []
    })
}

/// An administrator, a manager running a department of one, and an employee
/// outside it - with a token each for the two who may have one.
struct Cast {
    admin: Option<String>,
    manager: Option<String>,
    employee: Option<String>,
    lead_id: String,
    inside_id: String,
    outside_id: String,
    department_id: String,
}

async fn cast(server: &TestServer) -> Cast {
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
    let id_of = |email: &str| {
        users
            .as_array()
            .unwrap()
            .iter()
            .find(|user| user["email"] == email)
            .and_then(|user| user["id"].as_str())
            .expect("the account should exist")
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
    let department_id = body["id"].as_str().unwrap().to_string();
    let (status, _, body) = server
        .put_with_cookie(
            &format!("/api/v1/users/{inside_id}/department"),
            admin.as_deref(),
            json!({ "department_id": department_id }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let (_, manager, _) = server.login("lead@example.test", "correct horse").await;
    let (_, employee, _) = server.login("outside@example.test", "correct horse").await;

    Cast {
        admin,
        manager,
        employee,
        lead_id,
        inside_id,
        outside_id,
        department_id,
    }
}

/// Issues a token to whoever holds `cookie`, and answers it.
async fn issue(server: &TestServer, cookie: Option<&str>, name: &str) -> (String, String) {
    let (status, _, body) = server
        .post_with_cookie("/api/v1/read-tokens", cookie, json!({ "name": name, "scope": "hours" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    (body["id"].as_str().unwrap().to_string(), body["token"].as_str().unwrap().to_string())
}

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

/// A JSON dataset read with a token, as its rows.
async fn rows(server: &TestServer, token: &str, path: &str) -> Vec<Value> {
    let (status, body) = server.get_with_header(path, Some(&bearer(token))).await;
    assert_eq!(status, StatusCode::OK, "{path}: {body}");
    body.as_array().unwrap_or_else(|| panic!("{path} answers an array: {body}")).clone()
}

fn emails(rows: &[Value]) -> Vec<String> {
    let mut emails: Vec<String> = rows.iter().map(|row| row["email"].as_str().unwrap().to_string()).collect();
    emails.sort();
    emails.dedup();
    emails
}

/// The month every test reads: August 2026, long past whenever this runs.
const MONTH: &str = "from=2026-08-01&to=2026-08-31";

async fn record_the_month(server: &TestServer) {
    for (token, dates) in [
        ("inside-token", vec!["2026-08-24", "2026-08-25", "2026-08-26"]),
        ("outside-token", vec!["2026-08-24", "2026-08-27"]),
        ("lead-token", vec!["2026-08-25"]),
    ] {
        for date in dates {
            let (status, body) = server.post_day(token, day(date)).await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
    }
    let (status, body) = server.post_day("inside-token", leave("2026-08-27")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

// Who reads what -----------------------------------------------------------------

#[tokio::test]
async fn a_token_reads_exactly_what_its_owner_may_see() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;
    record_the_month(&server).await;

    let (_, admins) = issue(&server, cast.admin.as_deref(), "Company Grafana").await;
    let (_, managers) = issue(&server, cast.manager.as_deref(), "Team Grafana").await;

    // The administrator's token: everybody, the seeded employee included.
    let everyone = emails(&rows(&server, &admins, "/api/bi/v1/people").await);
    for email in ["boss@example.test", "inside@example.test", "outside@example.test", "lead@example.test"] {
        assert!(everyone.contains(&email.to_string()), "{email} missing from {everyone:?}");
    }

    // The manager's: themselves and their department, in every dataset that
    // names people - and nobody else's days, not even as empty rows.
    let mine = vec!["inside@example.test".to_string(), "lead@example.test".to_string()];
    assert_eq!(emails(&rows(&server, &managers, "/api/bi/v1/people").await), mine);
    assert_eq!(emails(&rows(&server, &managers, &format!("/api/bi/v1/days?{MONTH}")).await), mine);
    assert_eq!(emails(&rows(&server, &managers, &format!("/api/bi/v1/summary?{MONTH}")).await), mine);

    // Departments are listed whole, as they are to a manager on the screens.
    let departments = rows(&server, &managers, "/api/bi/v1/departments").await;
    assert_eq!(departments.len(), 1);
    assert_eq!(departments[0]["department_id"], cast.department_id.as_str());
    assert_eq!(departments[0]["manager_id"], cast.lead_id.as_str());
    assert_eq!(departments[0]["manager"], "lead");
}

#[tokio::test]
async fn only_someone_who_reads_the_team_gets_a_token() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;

    let (status, _, body) = server
        .post_with_cookie("/api/v1/read-tokens", cast.employee.as_deref(), json!({ "name": "mine", "scope": "hours" }))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body) = server.get_with_cookie("/api/v1/read-tokens", cast.employee.as_deref()).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    let (status, _, body) = server
        .post_with_cookie("/api/v1/read-tokens", cast.manager.as_deref(), json!({ "name": "   ", "scope": "hours" }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "a token needs a name: {body}");
}

#[tokio::test]
async fn a_token_stops_when_its_owner_may_no_longer_read() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;
    let (_, token) = issue(&server, cast.manager.as_deref(), "Team Grafana").await;
    let path = "/api/bi/v1/people";

    let (status, _) = server.get_with_header(path, Some(&bearer(&token))).await;
    assert_eq!(status, StatusCode::OK);

    // Back to employee: the screens refuse them the team, so does the token.
    let (status, _, _) = server
        .patch_with_cookie(&format!("/api/v1/users/{}", cast.lead_id), cast.admin.as_deref(), json!({ "role": "employee" }))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = server.get_with_header(path, Some(&bearer(&token))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // Made a manager again, it reads again: authority is read per request.
    let (status, _, _) = server
        .patch_with_cookie(&format!("/api/v1/users/{}", cast.lead_id), cast.admin.as_deref(), json!({ "role": "manager" }))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = server.get_with_header(path, Some(&bearer(&token))).await;
    assert_eq!(status, StatusCode::OK);

    // Deactivated: the token is as dead as an unknown one.
    let (status, _, _) = server
        .patch_with_cookie(&format!("/api/v1/users/{}", cast.lead_id), cast.admin.as_deref(), json!({ "active": false }))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = server.get_with_header(path, Some(&bearer(&token))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
}

#[tokio::test]
async fn a_revoked_token_reads_nothing_and_a_second_revoke_changes_nothing() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;
    let (id, token) = issue(&server, cast.manager.as_deref(), "Team Grafana").await;

    let (status, _, _) = server.delete_with_cookie(&format!("/api/v1/read-tokens/{id}"), cast.manager.as_deref()).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = server.get_with_header("/api/bi/v1/people", Some(&bearer(&token))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    let revoked_at: chrono::DateTime<chrono::Utc> = server.scalar("SELECT revoked_at FROM read_tokens").await;
    let (status, _, _) = server.delete_with_cookie(&format!("/api/v1/read-tokens/{id}"), cast.manager.as_deref()).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "revoking twice is still revoked");
    let again: chrono::DateTime<chrono::Utc> = server.scalar("SELECT revoked_at FROM read_tokens").await;
    assert_eq!(revoked_at, again, "when access ended is a fact a second click must not rewrite");
}

#[tokio::test]
async fn a_manager_sees_and_revokes_their_own_tokens_and_an_administrator_everyone_s() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;
    let (managers, _) = issue(&server, cast.manager.as_deref(), "Team Grafana").await;
    let (admins, _) = issue(&server, cast.admin.as_deref(), "Company Grafana").await;

    let (_, listed) = server.get_with_cookie("/api/v1/read-tokens", cast.manager.as_deref()).await;
    let listed = listed.as_array().unwrap();
    assert_eq!(listed.len(), 1, "a manager lists their own: {listed:?}");
    assert_eq!(listed[0]["id"], managers.as_str());
    assert_eq!(listed[0]["scope"], "hours");
    assert!(
        listed[0].get("token").is_none() && listed[0].get("token_hash").is_none(),
        "shown once, never listed"
    );

    let (_, everything) = server.get_with_cookie("/api/v1/read-tokens", cast.admin.as_deref()).await;
    assert_eq!(everything.as_array().unwrap().len(), 2, "an administrator lists every token");

    // Somebody else's token is one that does not exist, to a manager.
    let (status, _, _) = server
        .delete_with_cookie(&format!("/api/v1/read-tokens/{admins}"), cast.manager.as_deref())
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = server
        .delete_with_cookie(&format!("/api/v1/read-tokens/{managers}"), cast.admin.as_deref())
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "an administrator cleans up after anybody");
}

#[tokio::test]
async fn a_read_token_and_an_agent_token_open_different_doors() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;
    let (_, token) = issue(&server, cast.admin.as_deref(), "Company Grafana").await;

    // A read token cannot file a day under anybody's name...
    let (status, body) = server.post_day(&token, day("2026-08-24")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    // ...an agent's cannot read the team...
    let (status, body) = server.get_with_header("/api/bi/v1/people", Some("Bearer inside-token")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    // ...and a browser's session is not a bearer token at all.
    let (status, body) = server.get_with_cookie("/api/bi/v1/people", cast.admin.as_deref()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    // An unknown BI path is the API's 404, not the web UI's page.
    let (status, body) = server.get_with_header("/api/bi/v1/hours", Some(&bearer(&token))).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::NOT_FOUND, Some("no such endpoint")));
}

#[tokio::test]
async fn issuing_and_revoking_are_recorded_and_reading_is_not() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;
    let (id, token) = issue(&server, cast.manager.as_deref(), "Team Grafana").await;

    let issued: i64 = server.scalar("SELECT count(*) FROM audit_log WHERE action = 'read_token.issued'").await;
    assert_eq!(issued, 1);
    let label: String = server.scalar("SELECT target_label FROM audit_log WHERE action = 'read_token.issued'").await;
    assert_eq!(label, "Team Grafana");
    let leaked: i64 = server.scalar("SELECT count(*) FROM audit_log WHERE details::text LIKE '%kasl_read_%'").await;
    assert_eq!(leaked, 0, "the token itself is never written down");

    // A dashboard polls; the log must not fill with it. The token's last use
    // is what moves instead.
    let before = server.count("audit_log").await;
    assert!(
        server
            .optional_scalar::<chrono::DateTime<chrono::Utc>>("SELECT last_used_at FROM read_tokens")
            .await
            .is_none()
    );
    for _ in 0..3 {
        rows(&server, &token, &format!("/api/bi/v1/days?{MONTH}")).await;
    }
    assert_eq!(server.count("audit_log").await, before, "reads are not recorded one by one");
    assert!(
        server
            .optional_scalar::<chrono::DateTime<chrono::Utc>>("SELECT last_used_at FROM read_tokens")
            .await
            .is_some()
    );

    let (status, _, _) = server.delete_with_cookie(&format!("/api/v1/read-tokens/{id}"), cast.manager.as_deref()).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let revoked: i64 = server.scalar("SELECT count(*) FROM audit_log WHERE action = 'read_token.revoked'").await;
    assert_eq!(revoked, 1);
}

// The rows are the server's own ---------------------------------------------------

#[tokio::test]
async fn the_days_are_the_timesheet_s_cells() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;
    record_the_month(&server).await;
    // A short day and a holiday in the month, so the norm has more than one
    // value to agree on.
    let (status, _, body) = server
        .put_with_cookie(
            "/api/v1/calendar?year=2026",
            cast.admin.as_deref(),
            json!({ "days": [
                { "date": "2026-08-20", "kind": "short_day" },
                { "date": "2026-08-21", "kind": "holiday", "note": "A holiday" }
            ] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, token) = issue(&server, cast.admin.as_deref(), "Company Grafana").await;

    let (status, timesheet) = server.get_with_cookie("/api/v1/team/timesheet?month=2026-08", cast.admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{timesheet}");
    let days = rows(&server, &token, &format!("/api/bi/v1/days?{MONTH}")).await;

    let by_key: HashMap<(String, String), &Value> = days
        .iter()
        .map(|row| ((row["user_id"].as_str().unwrap().to_string(), row["date"].as_str().unwrap().to_string()), row))
        .collect();
    let mut compared = 0;
    for person in timesheet["rows"].as_array().unwrap() {
        for cell in person["days"].as_array().unwrap() {
            let key = (person["id"].as_str().unwrap().to_string(), cell["date"].as_str().unwrap().to_string());
            let row = by_key.get(&key).unwrap_or_else(|| panic!("no BI row for {key:?}"));
            for field in ["state", "kind", "worked_seconds", "norm_seconds", "filed", "report"] {
                assert_eq!(row[field], cell[field], "{field} of {key:?}");
            }
            assert_eq!(row["department_id"], person["department_id"], "{key:?}");
            compared += 1;
        }
    }
    assert_eq!(compared, days.len(), "one BI row per cell, and no more");
    // The cast, and the employee every test server is seeded with.
    assert_eq!(compared, 31 * 5, "every date of August for each of the five people");

    // What the cells do not carry, the day under them does.
    let worked = by_key[&(cast.inside_id.clone(), "2026-08-24".to_string())];
    assert_eq!(worked["started_at"], "2026-08-24T12:00:00Z", "in UTC");
    assert_eq!(worked["ended_at"], "2026-08-24T21:00:00Z");
    assert_eq!(
        (worked["worked_seconds"].as_i64(), worked["paused_seconds"].as_i64()),
        (Some(30_600), Some(1_800))
    );
    let holiday = by_key[&(cast.outside_id.clone(), "2026-08-21".to_string())];
    assert_eq!((holiday["calendar"].as_str(), holiday["state"].as_str()), (Some("holiday"), Some("free")));
    let saturday = by_key[&(cast.outside_id.clone(), "2026-08-22".to_string())];
    assert_eq!(saturday["weekend"], true);
    let unrecorded = by_key[&(cast.outside_id.clone(), "2026-08-25".to_string())];
    assert_eq!(
        (unrecorded["state"].as_str(), unrecorded["worked_seconds"].is_null()),
        (Some("unrecorded"), true),
        "never a zero"
    );
}

#[tokio::test]
async fn the_summary_is_the_team_table_s_rows() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;
    record_the_month(&server).await;
    let (_, token) = issue(&server, cast.admin.as_deref(), "Company Grafana").await;

    // A month over, and a fortnight running through today - where what has
    // come due and what the range asks part ways, and a summary that mixed
    // the two up would still agree about August.
    let today: chrono::NaiveDate = server.scalar("SELECT current_date").await;
    let running = format!("from={}&to={}", today - chrono::Days::new(3), today + chrono::Days::new(10));
    for range in [MONTH.to_string(), running.clone()] {
        let (status, team) = server.get_with_cookie(&format!("/api/v1/team/days?{range}"), cast.admin.as_deref()).await;
        assert_eq!(status, StatusCode::OK, "{team}");
        let summary = rows(&server, &token, &format!("/api/bi/v1/summary?{range}")).await;

        let members = team["members"].as_array().unwrap();
        assert_eq!(summary.len(), members.len());
        for (row, member) in summary.iter().zip(members) {
            assert_eq!(row["user_id"], member["id"]);
            for (bi, screen) in [
                ("days_worked", "days_recorded"),
                ("days_away", "days_away"),
                ("worked_seconds", "worked_seconds"),
                ("paused_seconds", "paused_seconds"),
                ("norm_seconds", "norm_seconds"),
                ("due_seconds", "due_seconds"),
                ("last_day", "last_day"),
                ("work_rate", "work_rate"),
            ] {
                assert_eq!(row[bi], member[screen], "{bi} of {} over {range}", member["email"]);
            }
        }
        if range == running {
            let first = &summary[0];
            assert!(
                first["due_seconds"].as_i64() < first["norm_seconds"].as_i64(),
                "a range running into the future owes less than it asks: {first}"
            );
        } else {
            let inside = summary.iter().find(|row| row["user_id"] == cast.inside_id.as_str()).unwrap();
            assert_eq!((inside["days_worked"].as_i64(), inside["days_away"].as_i64()), (Some(3), Some(1)));
        }
    }
}

#[tokio::test]
async fn the_csv_is_the_json_value_for_value() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;
    record_the_month(&server).await;
    let (_, token) = issue(&server, cast.admin.as_deref(), "Company Grafana").await;

    for (name, query, columns) in [
        ("people", "", kasl_server::bi::PEOPLE.as_slice()),
        ("departments", "", kasl_server::bi::DEPARTMENTS.as_slice()),
        ("days", MONTH, kasl_server::bi::DAYS.as_slice()),
        ("summary", MONTH, kasl_server::bi::SUMMARY.as_slice()),
    ] {
        let json = rows(&server, &token, &format!("/api/bi/v1/{name}?{query}")).await;
        let (status, headers, csv) = server
            .get_file_with_header(&format!("/api/bi/v1/{name}.csv?{query}"), Some(&bearer(&token)))
            .await;
        assert_eq!(status, StatusCode::OK, "{name}.csv");
        assert_eq!(headers[header::CONTENT_TYPE], "text/csv; charset=utf-8");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store", "people's hours stay out of caches");

        // The fixtures hold no commas, quotes or line breaks, so a split is a
        // parse; the quoting is unit-tested beside the writer.
        let text = String::from_utf8(csv).unwrap();
        let mut lines = text.split("\r\n").filter(|line| !line.is_empty());
        assert_eq!(lines.next().unwrap(), columns.join(","), "{name}: the header is the column list");
        let lines: Vec<&str> = lines.collect();
        assert_eq!(lines.len(), json.len(), "{name}: a line per row");

        for (line, row) in lines.iter().zip(&json) {
            let object = row.as_object().unwrap();
            assert_eq!(object.len(), columns.len(), "{name}: a key per column");
            for (field, column) in line.split(',').zip(columns.iter()) {
                let expected = match &row[*column] {
                    Value::Null => String::new(),
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                };
                assert_eq!(field, expected, "{name}.{column}");
            }
        }
    }
}

#[tokio::test]
async fn a_range_is_checked_the_way_every_range_is() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;
    let (_, token) = issue(&server, cast.admin.as_deref(), "Company Grafana").await;

    for path in ["/api/bi/v1/days", "/api/bi/v1/summary.csv"] {
        let (status, body) = server
            .get_with_header(&format!("{path}?from=2026-08-31&to=2026-08-01"), Some(&bearer(&token)))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}: {body}");
        let (status, body) = server
            .get_with_header(&format!("{path}?from=2025-01-01&to=2026-08-01"), Some(&bearer(&token)))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}: more than 400 days: {body}");
    }
}

// What the people read about are told --------------------------------------------

#[tokio::test]
async fn the_privacy_manifest_names_every_token_that_reads_other_people() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;

    let (status, manifest) = server.get_with_cookie("/api/v1/privacy", cast.employee.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{manifest}");
    assert_eq!(manifest["read_elsewhere"], json!([]), "always present, and empty when nothing reads them");

    let (managers, _) = issue(&server, cast.manager.as_deref(), "Team Grafana").await;
    issue(&server, cast.admin.as_deref(), "Company Grafana").await;

    // Told to an employee, and to an agent - the CLI shows the same page.
    let (_, manifest) = server.get_with_cookie("/api/v1/privacy", cast.employee.as_deref()).await;
    let (_, from_agent) = server.get_with_header("/api/v1/privacy/agent", Some("Bearer outside-token")).await;
    assert_eq!(manifest["read_elsewhere"], from_agent["read_elsewhere"]);
    let read = manifest["read_elsewhere"].as_array().unwrap();
    assert_eq!(read.len(), 2, "{read:?}");
    let by = |through: &str| {
        read.iter()
            .find(|entry| entry["through"] == through)
            .unwrap_or_else(|| panic!("{through} missing: {read:?}"))
    };
    assert_eq!(by("Team Grafana")["about"], "people in Engineering");
    assert_eq!(by("Team Grafana")["by"], "lead, a manager");
    assert_eq!(by("Company Grafana")["about"], "everyone");
    assert!(!format!("{read:?}").contains("kasl_read_"), "the manifest never carries a token");

    // Revoked, it reads nobody, and the manifest stops saying it does.
    server
        .delete_with_cookie(&format!("/api/v1/read-tokens/{managers}"), cast.manager.as_deref())
        .await;
    let (_, manifest) = server.get_with_cookie("/api/v1/privacy", cast.employee.as_deref()).await;
    assert_eq!(manifest["read_elsewhere"].as_array().unwrap().len(), 1);

    // A manager who runs nothing reads only themselves: not other people's
    // business, so not on their page.
    let (status, _, _) = server
        .delete_with_cookie(&format!("/api/v1/departments/{}", cast.department_id), cast.admin.as_deref())
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    issue(&server, cast.manager.as_deref(), "Own Grafana").await;
    let (_, manifest) = server.get_with_cookie("/api/v1/privacy", cast.employee.as_deref()).await;
    let read = manifest["read_elsewhere"].as_array().unwrap();
    assert!(!read.iter().any(|entry| entry["through"] == "Own Grafana"), "{read:?}");
}
