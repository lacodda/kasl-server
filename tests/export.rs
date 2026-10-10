//! Hours as files, against a live database (ADR 0023).
//!
//! Two kinds of claim are tested here. Who may download whose hours - the same
//! rule the team table follows, so the cases are the same cast of an employee,
//! a manager with a department and an administrator. And that the files say
//! what the screen says: the summary row of a person is their row in
//! `/team/days`, and the days of a person add up to their summary, the worked
//! hours and the norm alike.

mod support;

use std::{collections::HashMap, io::Read};

use axum::http::{StatusCode, header};
use serde_json::{Value, json};
use support::TestServer;

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

/// The cast: an administrator, a manager with a department of one, and an
/// employee outside it.
struct Cast {
    admin: Option<String>,
    manager: Option<String>,
    employee: Option<String>,
    inside_id: String,
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
        inside_id,
    }
}

/// The week every test exports: Monday the 24th to Sunday the 30th.
const WEEK: &str = "from=2026-08-24&to=2026-08-30";

/// A CSV as rows of header -> value. The fixtures hold no commas, quotes or
/// line breaks, so a split is a parse; the quoting rules are unit-tested next
/// to the writer.
fn parse_csv(bytes: &[u8]) -> Vec<HashMap<String, String>> {
    let text = std::str::from_utf8(bytes).expect("a CSV is UTF-8");
    assert!(text.ends_with("\r\n"), "every line ends in CRLF, the last one too: {text:?}");
    let mut lines = text.split("\r\n").filter(|line| !line.is_empty());
    let header: Vec<&str> = lines.next().expect("a header row").split(',').collect();
    lines
        .map(|line| {
            let fields: Vec<&str> = line.split(',').collect();
            assert_eq!(fields.len(), header.len(), "a field per column: {line}");
            header.iter().map(|name| name.to_string()).zip(fields.into_iter().map(str::to_string)).collect()
        })
        .collect()
}

/// Sums an hours column, in hundredths, so two-place figures add exactly.
fn hundredths(rows: &[&HashMap<String, String>], column: &str) -> i64 {
    rows.iter()
        .filter(|row| !row[column].is_empty())
        .map(|row| {
            let (whole, fraction) = row[column].split_once('.').expect("hours to two places");
            whole.parse::<i64>().unwrap() * 100 + fraction.parse::<i64>().unwrap()
        })
        .sum()
}

#[tokio::test]
async fn a_manager_exports_the_people_they_see_and_nobody_else() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;

    let (status, headers, body) = server
        .get_file_with_cookie(&format!("/api/v1/team/export/summary.csv?{WEEK}"), cast.manager.as_deref())
        .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(headers[header::CONTENT_TYPE], "text/csv; charset=utf-8");
    assert_eq!(
        headers[header::CONTENT_DISPOSITION],
        "attachment; filename=\"kasl-team-2026-08-24-to-2026-08-30-summary.csv\""
    );
    assert_eq!(headers[header::CACHE_CONTROL], "no-store", "people's hours are not left in a cache");

    let rows = parse_csv(&body);
    let emails: Vec<&str> = rows.iter().map(|row| row["email"].as_str()).collect();
    assert_eq!(emails.len(), 2, "{emails:?}");
    assert!(emails.contains(&"inside@example.test"), "their department: {emails:?}");
    assert!(emails.contains(&"lead@example.test"), "themselves: {emails:?}");
    // The absences are the point, as on the screen (ADR 0009).
    assert!(!emails.contains(&"outside@example.test"), "{emails:?}");
    assert!(!emails.contains(&"boss@example.test"), "{emails:?}");

    // The days file is held to the same rule.
    let (status, _, body) = server
        .get_file_with_cookie(&format!("/api/v1/team/export/days.csv?{WEEK}"), cast.manager.as_deref())
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        parse_csv(&body).iter().all(|row| row["email"] != "outside@example.test"),
        "nobody outside the department, on any date"
    );

    server.close().await;
}

#[tokio::test]
async fn the_summary_says_what_the_team_table_says() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;

    for date in ["2026-08-25", "2026-08-26"] {
        let (status, body) = server.post_day("inside-token", day(date)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let (status, body) = server.post_day("inside-token", leave("2026-08-27")).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (_, table) = server.get_with_cookie(&format!("/api/v1/team/days?{WEEK}"), cast.admin.as_deref()).await;
    let (status, _, body) = server
        .get_file_with_cookie(&format!("/api/v1/team/export/summary.csv?{WEEK}"), cast.admin.as_deref())
        .await;
    assert_eq!(status, StatusCode::OK);
    let rows = parse_csv(&body);

    let members = table["members"].as_array().unwrap();
    assert_eq!(rows.len(), members.len(), "a row per person on the screen");
    for member in members {
        let row = rows
            .iter()
            .find(|row| row["email"] == member["email"])
            .expect("everyone on the screen is in the file");
        let hours = |seconds: &Value| format!("{:.2}", seconds.as_i64().unwrap() as f64 / 3600.0);
        assert_eq!(row["person"], member["display_name"].as_str().unwrap());
        assert_eq!(row["worked_hours"], hours(&member["worked_seconds"]), "{row:?}");
        assert_eq!(row["paused_hours"], hours(&member["paused_seconds"]), "{row:?}");
        assert_eq!(row["norm_hours"], hours(&member["norm_seconds"]), "{row:?}");
        assert_eq!(row["days_worked"], member["days_recorded"].to_string());
        assert_eq!(row["days_away"], member["days_away"].to_string());
    }

    // And the figures themselves, so the comparison above is not two empty
    // columns agreeing: two days of 8.5 hours, a day of leave that owes
    // nothing, and four days of eight still due.
    let inside = rows.iter().find(|row| row["email"] == "inside@example.test").unwrap();
    assert_eq!(inside["worked_hours"], "17.00");
    assert_eq!(inside["paused_hours"], "1.00");
    assert_eq!(inside["norm_hours"], "32.00");
    assert_eq!(inside["days_worked"], "2");
    assert_eq!(inside["days_away"], "1");
    assert_eq!(inside["department"], "Engineering");
    assert_eq!(inside["active"], "true");
    assert_eq!(inside["work_rate"], "1");

    server.close().await;
}

#[tokio::test]
async fn the_days_add_up_to_the_summary() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;

    // Tuesday worked, Thursday on leave, Saturday worked though nothing was
    // due. Monday, Wednesday and Friday were due and never recorded.
    for upload in [day("2026-08-25"), leave("2026-08-27"), day("2026-08-29")] {
        let (status, body) = server.post_day("inside-token", upload).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    let (_, _, summary) = server
        .get_file_with_cookie(&format!("/api/v1/team/export/summary.csv?{WEEK}"), cast.admin.as_deref())
        .await;
    let (status, _, days) = server
        .get_file_with_cookie(&format!("/api/v1/team/export/days.csv?{WEEK}"), cast.admin.as_deref())
        .await;
    assert_eq!(status, StatusCode::OK);
    let summary = parse_csv(&summary);
    let days = parse_csv(&days);

    let theirs: Vec<&HashMap<String, String>> = days.iter().filter(|row| row["email"] == "inside@example.test").collect();
    let dates: Vec<&str> = theirs.iter().map(|row| row["date"].as_str()).collect();
    assert_eq!(
        dates,
        ["2026-08-24", "2026-08-25", "2026-08-26", "2026-08-27", "2026-08-28", "2026-08-29"],
        "every date recorded or due, in order - and Sunday, neither, left out"
    );

    let by_date = |date: &str| *theirs.iter().find(|row| row["date"] == date).unwrap();
    let monday = by_date("2026-08-24");
    assert_eq!(monday["kind"], "", "due and not recorded: no kind, no figures");
    assert_eq!(monday["worked_hours"], "");
    assert_eq!(monday["norm_hours"], "8.00");
    let tuesday = by_date("2026-08-25");
    assert_eq!(tuesday["kind"], "work");
    assert_eq!(tuesday["started_at"], "2026-08-25T12:00:00Z", "an instant, in UTC");
    assert_eq!(tuesday["ended_at"], "2026-08-25T21:00:00Z");
    assert_eq!(tuesday["worked_hours"], "8.50");
    assert_eq!(tuesday["paused_hours"], "0.50");
    assert_eq!(by_date("2026-08-27")["kind"], "vacation");
    assert_eq!(by_date("2026-08-27")["norm_hours"], "0.00", "leave owes nothing");
    assert_eq!(by_date("2026-08-29")["norm_hours"], "0.00", "a Saturday owes nothing, worked or not");

    let row = summary.iter().find(|row| row["email"] == "inside@example.test").unwrap();
    for column in ["worked_hours", "paused_hours", "norm_hours"] {
        assert_eq!(
            hundredths(&theirs, column),
            hundredths(&[row], column),
            "{column}: the days add up to the summary"
        );
    }

    // Somebody with nothing recorded still owes the week, day by day.
    let lead: Vec<&HashMap<String, String>> = days.iter().filter(|row| row["email"] == "lead@example.test").collect();
    assert_eq!(lead.len(), 5, "Monday to Friday, each due");

    server.close().await;
}

#[tokio::test]
async fn an_employee_may_export_only_themselves() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;
    let (status, body) = server.post_day("outside-token", day("2026-08-25")).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    for path in ["/api/v1/team/export.xlsx", "/api/v1/team/export/summary.csv", "/api/v1/team/export/days.csv"] {
        let (status, _, _) = server.get_file_with_cookie(&format!("{path}?{WEEK}"), cast.employee.as_deref()).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
    }

    let (status, headers, body) = server
        .get_file_with_cookie(&format!("/api/v1/me/export/summary.csv?{WEEK}"), cast.employee.as_deref())
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers[header::CONTENT_DISPOSITION],
        "attachment; filename=\"kasl-me-2026-08-24-to-2026-08-30-summary.csv\""
    );
    let rows = parse_csv(&body);
    assert_eq!(rows.len(), 1, "one row of people: themselves");
    assert_eq!(rows[0]["email"], "outside@example.test");
    assert_eq!(rows[0]["worked_hours"], "8.50");

    let (_, _, body) = server
        .get_file_with_cookie(&format!("/api/v1/me/export/days.csv?{WEEK}"), cast.employee.as_deref())
        .await;
    assert!(parse_csv(&body).iter().all(|row| row["email"] == "outside@example.test"));

    // A manager's own export is theirs too, not their department's.
    let (_, _, body) = server
        .get_file_with_cookie(&format!("/api/v1/me/export/summary.csv?{WEEK}"), cast.manager.as_deref())
        .await;
    let rows = parse_csv(&body);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["email"], "lead@example.test");

    server.close().await;
}

#[tokio::test]
async fn the_workbook_holds_both_tables() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;
    let (status, body) = server.post_day("inside-token", day("2026-08-25")).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, headers, body) = server
        .get_file_with_cookie(&format!("/api/v1/team/export.xlsx?{WEEK}"), cast.manager.as_deref())
        .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(
        headers[header::CONTENT_TYPE],
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    );
    assert_eq!(
        headers[header::CONTENT_DISPOSITION],
        "attachment; filename=\"kasl-team-2026-08-24-to-2026-08-30.xlsx\""
    );

    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(body)).expect("a workbook is a zip");
    let mut part = |name: &str| {
        let mut text = String::new();
        archive
            .by_name(name)
            .unwrap_or_else(|_| panic!("the workbook should have {name}"))
            .read_to_string(&mut text)
            .unwrap();
        text
    };

    let workbook = part("xl/workbook.xml");
    assert!(workbook.contains(r#"name="Summary""#), "{workbook}");
    assert!(workbook.contains(r#"name="Days""#), "{workbook}");
    // Every header and every name is a shared string. The people the manager
    // may see are in the file, the one they may not is not.
    let strings = part("xl/sharedStrings.xml");
    for expected in [
        "Worked (h)",
        "Norm (h)",
        "Started (UTC)",
        "inside@example.test",
        "lead@example.test",
        "Engineering",
    ] {
        assert!(strings.contains(expected), "{expected} should be in the workbook: {strings}");
    }
    assert!(!strings.contains("outside@example.test"), "{strings}");
    // Hours are numbers a spreadsheet can sum, not text: the summary sheet
    // holds Tuesday's 8.5 as a value.
    let summary = part("xl/worksheets/sheet1.xml");
    assert!(summary.contains("<v>8.5</v>"), "{summary}");

    // And the personal one is a workbook of the same shape.
    let (status, headers, _) = server
        .get_file_with_cookie(&format!("/api/v1/me/export.xlsx?{WEEK}"), cast.employee.as_deref())
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers[header::CONTENT_DISPOSITION],
        "attachment; filename=\"kasl-me-2026-08-24-to-2026-08-30.xlsx\""
    );

    server.close().await;
}

#[tokio::test]
async fn exporting_other_people_is_audited_and_exporting_yourself_is_not() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;

    server
        .get_file_with_cookie(&format!("/api/v1/me/export/days.csv?{WEEK}"), cast.manager.as_deref())
        .await;
    assert_eq!(
        server.scalar::<i64>("SELECT count(*) FROM audit_log WHERE action = 'hours.exported'").await,
        0,
        "your own hours are nobody else's record"
    );

    let (status, _, _) = server
        .get_file_with_cookie(&format!("/api/v1/team/export.xlsx?{WEEK}"), cast.manager.as_deref())
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, entries) = server.get_with_cookie("/api/v1/audit", cast.admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{entries}");
    let entry = entries
        .as_array()
        .expect("a list of entries")
        .iter()
        .find(|entry| entry["action"] == "hours.exported")
        .unwrap_or_else(|| panic!("the export should be in the audit log: {entries}"))
        .clone();
    assert_eq!(entry["actor_email"], "lead@example.test");
    assert_eq!(entry["details"]["file"], "workbook");
    assert_eq!(entry["details"]["from"], "2026-08-24");
    assert_eq!(entry["details"]["to"], "2026-08-30");
    assert_eq!(entry["details"]["people"], 2);

    server.close().await;
}

#[tokio::test]
async fn signing_in_is_required_and_the_range_is_checked() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;

    for path in ["/api/v1/team/export.xlsx", "/api/v1/me/export/days.csv"] {
        let (status, _, _) = server.get_file_with_cookie(&format!("{path}?{WEEK}"), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}");
    }
    for query in ["", "?from=2026-08-30&to=2026-08-24", "?from=2026-01-01&to=2027-06-01"] {
        let (status, _, _) = server
            .get_file_with_cookie(&format!("/api/v1/team/export/summary.csv{query}"), cast.admin.as_deref())
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "`{query}` should be refused");
    }

    server.close().await;
}

#[tokio::test]
async fn a_name_a_spreadsheet_would_run_is_defused() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;

    let (status, _, body) = server
        .patch_with_cookie(
            &format!("/api/v1/users/{}", cast.inside_id),
            cast.admin.as_deref(),
            json!({ "display_name": "=HYPERLINK(\"http://example.test\")" }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let (_, _, body) = server
        .get_file_with_cookie(&format!("/api/v1/team/export/summary.csv?{WEEK}"), cast.admin.as_deref())
        .await;
    let text = String::from_utf8(body).unwrap();
    assert!(
        text.contains("\"'=HYPERLINK(\"\"http://example.test\"\")\""),
        "prefixed, then quoted for its quotes: {text}"
    );

    server.close().await;
}

#[tokio::test]
async fn someone_who_left_is_in_the_periods_they_worked() {
    let Some(server) = TestServer::start().await else { return };
    let cast = cast(&server).await;
    let (status, body) = server.post_day("inside-token", day("2026-08-25")).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, _, body) = server
        .patch_with_cookie(&format!("/api/v1/users/{}", cast.inside_id), cast.admin.as_deref(), json!({ "active": false }))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let (_, _, body) = server
        .get_file_with_cookie(&format!("/api/v1/team/export/summary.csv?{WEEK}"), cast.manager.as_deref())
        .await;
    let rows = parse_csv(&body);
    let inside = rows
        .iter()
        .find(|row| row["email"] == "inside@example.test")
        .expect("their August hours are still August's");
    assert_eq!(inside["active"], "false");
    assert_eq!(inside["worked_hours"], "8.50");

    // A week they have nothing in does not list them.
    let (_, _, body) = server
        .get_file_with_cookie("/api/v1/team/export/summary.csv?from=2026-08-31&to=2026-09-06", cast.manager.as_deref())
        .await;
    assert!(parse_csv(&body).iter().all(|row| row["email"] != "inside@example.test"));

    server.close().await;
}
