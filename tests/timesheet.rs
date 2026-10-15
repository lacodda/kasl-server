//! The month as a record, against a live database (ADR 0024).
//!
//! Three kinds of claim. What each date of a month was - worked, open, away,
//! owed and unrecorded, ahead, owed nothing - decided by the server and held
//! here date by date. That a row adds up to the summary of the same month, the
//! hours and the norm alike, because a timesheet that disagreed with the team
//! table would make one of them a lie. And when a report counts as on time:
//! before the person's next working day began.

mod support;

use std::io::Read;

use axum::http::{StatusCode, header};
use serde_json::{Value, json};
use support::TestServer;

/// A finished day of nine hours with half an hour of lunch in it, at -03:00:
/// it begins at 12:00 UTC.
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

const FULL_DAY: i64 = 8 * 3600;

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

fn open(date: &str) -> Value {
    let mut day = day(date);
    day.as_object_mut().expect("an object").remove("ended_at");
    day
}

/// An administrator, a manager with a department of one, the employee in it,
/// and an employee outside it - the cast every team route is tested with.
struct Team {
    admin: Option<String>,
    manager: Option<String>,
    employee: Option<String>,
    outsider: Option<String>,
    inside_id: String,
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
    let department_id = body["id"].as_str().expect("an id").to_string();
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
    let (_, outsider, _) = server.login("outside@example.test", "correct horse").await;

    Team {
        admin,
        manager,
        employee,
        outsider,
        inside_id,
    }
}

/// August 2026, as `inside` lived it. The 1st is a Saturday and the 31st a
/// Monday the calendar makes a holiday.
///
/// | date | what | report |
/// |---|---|---|
/// | Mon 3 | worked | that evening - on time |
/// | Tue 4 | worked | the next afternoon - late |
/// | Wed 5 | worked | on Friday, while away - on time: leave is not a working day |
/// | Thu 6 | vacation | - |
/// | Sat 8 | worked, on a weekend | never, and Monday began - missing |
/// | Mon 10 | open, never closed | - |
/// | Fri 28 | worked, the last day | never, nothing begun since - pending |
///
/// Every other weekday is owed and unrecorded.
async fn august(server: &TestServer, team: &Team) {
    let (status, _, body) = server
        .put_with_cookie(
            "/api/v1/calendar?year=2026",
            team.admin.as_deref(),
            json!({ "days": [{ "date": "2026-08-31", "kind": "holiday", "note": "Summer holiday" }] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    for fixture in [
        day("2026-08-03"),
        day("2026-08-04"),
        day("2026-08-05"),
        leave("2026-08-06"),
        day("2026-08-08"),
        open("2026-08-10"),
        day("2026-08-28"),
    ] {
        let (status, body) = server.post_day("inside-token", fixture).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    // Reported now, and moved back to the moment the story says: a report is
    // written at the time it is received, and these days are two months old.
    for (date, at) in [
        ("2026-08-03", "2026-08-03T22:00:00Z"),
        ("2026-08-04", "2026-08-05T13:00:00Z"),
        ("2026-08-05", "2026-08-07T15:00:00Z"),
    ] {
        let (status, _, body) = server
            .post_with_cookie("/api/v1/me/reports", team.employee.as_deref(), json!({ "date": date }))
            .await;
        assert!(status.is_success(), "{body}");
        sqlx::query("UPDATE reports SET submitted_at = $2::timestamptz WHERE date = $1::date")
            .bind(date)
            .bind(at)
            .execute(&server.pool)
            .await
            .expect("the report should move");
    }
}

fn row<'a>(body: &'a Value, name: &str) -> &'a Value {
    body["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|row| row["display_name"] == name)
        .unwrap_or_else(|| panic!("{name} should have a row: {body}"))
}

fn names(body: &Value) -> Vec<&str> {
    body["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row["display_name"].as_str().expect("a name"))
        .collect()
}

fn cell<'a>(row: &'a Value, date: &str) -> &'a Value {
    row["days"]
        .as_array()
        .expect("days")
        .iter()
        .find(|cell| cell["date"] == date)
        .unwrap_or_else(|| panic!("{date} should be a cell: {row}"))
}

#[tokio::test]
async fn every_date_of_the_month_says_what_it_was() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;
    august(&server, &team).await;

    let (status, body) = server.get_with_cookie("/api/v1/team/timesheet?month=2026-08", team.admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["from"], "2026-08-01");
    assert_eq!(body["to"], "2026-08-31");

    let dates = body["dates"].as_array().expect("dates");
    assert_eq!(dates.len(), 31, "every date of August, and no more");
    assert_eq!(dates[30]["calendar"], "holiday");
    assert_eq!(dates[30]["note"], "Summer holiday");
    assert_eq!(dates[30]["hours"], 0.0);
    assert_eq!(dates[2]["hours"], 8.0, "a Monday is a full day");
    assert_eq!(dates[0]["weekend"], true, "the 1st is a Saturday");

    let inside = row(&body, "inside");
    assert_eq!(inside["days"].as_array().unwrap().len(), 31);

    let expect = |date: &str, state: &str, filed: Value| {
        let cell = cell(inside, date);
        assert_eq!(cell["state"], state, "{date}: {cell}");
        assert_eq!(cell["filed"], filed, "{date}: {cell}");
    };
    expect("2026-08-03", "worked", json!("on_time"));
    expect("2026-08-04", "worked", json!("late"));
    expect("2026-08-05", "worked", json!("on_time"));
    expect("2026-08-06", "away", Value::Null);
    expect("2026-08-07", "unrecorded", json!("missing"));
    expect("2026-08-08", "worked", json!("missing"));
    expect("2026-08-09", "free", Value::Null);
    expect("2026-08-10", "open", Value::Null);
    expect("2026-08-28", "worked", json!("pending"));
    expect("2026-08-31", "free", Value::Null);

    assert_eq!(cell(inside, "2026-08-03")["worked_seconds"], WORKED);
    assert_eq!(cell(inside, "2026-08-03")["report"], "submitted");
    assert_eq!(cell(inside, "2026-08-06")["kind"], "vacation");
    assert_eq!(cell(inside, "2026-08-06")["norm_seconds"], 0, "leave owes nothing");
    assert_eq!(cell(inside, "2026-08-07")["worked_seconds"], Value::Null, "absent, never a zero");
    assert_eq!(cell(inside, "2026-08-08")["norm_seconds"], 0, "a Saturday owes nothing, even worked");
    assert_eq!(cell(inside, "2026-08-10")["worked_seconds"], Value::Null, "an open day has no total");

    // Twenty weekdays are owed once the 31st is a holiday; six of them hold a
    // day, so fourteen hold nothing.
    assert_eq!(
        inside["counts"],
        json!({
            "vacation": 1, "sick": 0, "day_off": 0,
            "unrecorded": 14,
            "on_time": 2, "late": 1, "missing": 15, "pending": 1,
            "approved": 0
        })
    );

    server.close().await;
}

#[tokio::test]
async fn a_row_adds_up_to_the_summary_of_the_same_month() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;
    august(&server, &team).await;

    let (_, timesheet) = server.get_with_cookie("/api/v1/team/timesheet?month=2026-08", team.admin.as_deref()).await;
    let (_, table) = server
        .get_with_cookie("/api/v1/team/days?from=2026-08-01&to=2026-08-31", team.admin.as_deref())
        .await;

    let members = table["members"].as_array().expect("members");
    assert_eq!(names(&timesheet).len(), members.len(), "the same people: {timesheet}");

    for member in members {
        let name = member["display_name"].as_str().unwrap();
        let row = row(&timesheet, name);
        // The summary's own fields, unchanged: the row is the summary.
        for field in ["worked_seconds", "norm_seconds", "due_seconds", "days_recorded", "days_away"] {
            assert_eq!(row[field], member[field], "{name}'s {field}");
        }

        // And the cells add up to it, so the grid and its totals cannot part.
        let cells = row["days"].as_array().unwrap();
        let sum = |field: &str| cells.iter().filter_map(|cell| cell[field].as_i64()).sum::<i64>();
        assert_eq!(sum("norm_seconds"), member["norm_seconds"].as_i64().unwrap(), "{name}'s norm");
        assert_eq!(sum("worked_seconds"), member["worked_seconds"].as_i64().unwrap(), "{name}'s hours");
    }

    let inside = row(&timesheet, "inside");
    assert_eq!(inside["norm_seconds"], 19 * FULL_DAY, "twenty owed weekdays, one of them on leave");
    assert_eq!(inside["worked_seconds"], 5 * WORKED, "five finished days; the open one adds nothing");
    // August is over, so all of it has come due.
    assert_eq!(inside["due_seconds"], inside["norm_seconds"]);

    server.close().await;
}

#[tokio::test]
async fn a_report_corrected_later_keeps_its_timeliness_and_an_approval_is_counted() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;
    august(&server, &team).await;

    // Monday's report is approved.
    let (status, _, body) = server
        .put_with_cookie("/api/v1/reports/approval", team.admin.as_deref(), json!({ "enabled": true }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let monday: String = sqlx::query_scalar("SELECT id::text FROM reports WHERE date = '2026-08-03'")
        .fetch_one(&server.pool)
        .await
        .expect("Monday was reported");
    let (status, _, body) = server
        .post_with_cookie("/api/v1/reports/approve", team.manager.as_deref(), json!({ "ids": [monday] }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Wednesday, reported in time, is corrected in kasl and reported again
    // today - long after the next working day began. Timeliness is the first
    // report's: a correction does not make a day reported in time late.
    let mut corrected = day("2026-08-05");
    corrected["ended_at"] = json!("2026-08-05T17:00:00-03:00");
    let (status, body) = server.post_day("inside-token", corrected).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _, body) = server
        .post_with_cookie("/api/v1/me/reports", team.employee.as_deref(), json!({ "date": "2026-08-05" }))
        .await;
    assert!(status.is_success(), "{body}");
    let reports: i64 = sqlx::query_scalar("SELECT count(*) FROM reports WHERE date = '2026-08-05'")
        .fetch_one(&server.pool)
        .await
        .unwrap();
    assert_eq!(reports, 2, "the correction is a second report, not an edit of the first");

    let (_, timesheet) = server.get_with_cookie("/api/v1/team/timesheet?month=2026-08", team.manager.as_deref()).await;
    assert_eq!(timesheet["day_approval"], true);
    let inside = row(&timesheet, "inside");

    assert_eq!(cell(inside, "2026-08-03")["filed"], "on_time");
    assert_eq!(cell(inside, "2026-08-03")["report"], "approved", "the day still comes to what was approved");
    assert_eq!(cell(inside, "2026-08-05")["filed"], "on_time");
    assert_eq!(
        cell(inside, "2026-08-05")["report"],
        "submitted",
        "the newest report stands for the day as it is now"
    );
    assert_eq!(cell(inside, "2026-08-04")["filed"], "late");
    assert_eq!(inside["counts"]["approved"], 1);

    server.close().await;
}

#[tokio::test]
async fn a_month_ahead_is_owed_and_not_yet_due() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    let (status, body) = server.get_with_cookie("/api/v1/team/timesheet?month=2099-03", team.admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let inside = row(&body, "inside");

    let states: Vec<&str> = inside["days"].as_array().unwrap().iter().map(|cell| cell["state"].as_str().unwrap()).collect();
    assert!(states.iter().all(|state| matches!(*state, "ahead" | "free")), "{states:?}");
    assert!(
        inside["days"].as_array().unwrap().iter().all(|cell| cell["filed"].is_null()),
        "nothing is late yet"
    );
    assert_eq!(inside["due_seconds"], 0);
    assert!(inside["norm_seconds"].as_i64().unwrap() > 0, "but the month is owed");

    server.close().await;
}

#[tokio::test]
async fn a_manager_sees_their_department_and_an_employee_only_themselves() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    let (status, body) = server.get_with_cookie("/api/v1/team/timesheet?month=2026-08", team.manager.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut seen = names(&body);
    seen.sort();
    assert_eq!(seen, ["inside", "lead"], "their department and themselves - nobody else (ADR 0009)");

    let (status, _) = server.get_with_cookie("/api/v1/team/timesheet?month=2026-08", team.employee.as_deref()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) = server
        .get_file_with_cookie("/api/v1/team/timesheet.xlsx?month=2026-08", team.employee.as_deref())
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, body) = server.get_with_cookie("/api/v1/me/timesheet?month=2026-08", team.outsider.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(names(&body), ["outside"], "their own month, one row");

    let (status, _) = server.get_with_cookie("/api/v1/me/timesheet?month=2026-08", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    server.close().await;
}

#[tokio::test]
async fn a_month_that_is_not_a_month_is_refused() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    for bad in ["2026-8", "2026-08-15", "August"] {
        let (status, body) = server
            .get_with_cookie(&format!("/api/v1/team/timesheet?month={bad}"), team.admin.as_deref())
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
    }

    server.close().await;
}

#[tokio::test]
async fn somebody_who_left_stays_on_the_months_they_worked() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    let (status, body) = server.post_day("inside-token", day("2026-08-03")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _, body) = server
        .patch_with_cookie(&format!("/api/v1/users/{}", team.inside_id), team.admin.as_deref(), json!({ "active": false }))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    // August is a record of what happened in it: their day is in it, and the
    // total of August must not shrink in October because they have gone.
    let (_, august) = server.get_with_cookie("/api/v1/team/timesheet?month=2026-08", team.admin.as_deref()).await;
    let inside = row(&august, "inside");
    assert_eq!(inside["active"], false);
    assert_eq!(cell(inside, "2026-08-03")["state"], "worked");

    // A month they did not work is not theirs.
    let (_, september) = server.get_with_cookie("/api/v1/team/timesheet?month=2026-09", team.admin.as_deref()).await;
    assert!(!names(&september).contains(&"inside"), "{september}");

    server.close().await;
}

#[tokio::test]
async fn the_shape_of_a_month_is_the_record_of_it() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;
    august(&server, &team).await;

    let (_, timesheet) = server.get_with_cookie("/api/v1/team/timesheet?month=2026-08", team.admin.as_deref()).await;
    let (status, heatmap) = server.get_with_cookie("/api/v1/team/heatmap?month=2026-08", team.admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{heatmap}");

    assert_eq!(names(&heatmap), names(&timesheet), "the same people, in the same order");
    let shape = row(&heatmap, "inside");
    let record = row(&timesheet, "inside");
    assert_eq!(shape["worked_seconds"], record["worked_seconds"]);

    let recorded: Vec<&Value> = record["days"].as_array().unwrap().iter().filter(|cell| !cell["kind"].is_null()).collect();
    let squares = shape["days"].as_array().unwrap();
    assert_eq!(squares.len(), recorded.len(), "a square for every recorded date, and only those");
    let vacation = squares.iter().find(|square| square["date"] == "2026-08-06").expect("the day away is a square");
    assert_eq!(vacation["kind"], "vacation", "and says it was not worked");
    assert_eq!(shape["busiest_seconds"], WORKED, "the busiest day is a day of work");

    server.close().await;
}

#[tokio::test]
async fn the_timesheet_downloads_as_the_grid_and_is_audited() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;
    august(&server, &team).await;

    let (status, headers, body) = server
        .get_file_with_cookie("/api/v1/team/timesheet.xlsx?month=2026-08", team.manager.as_deref())
        .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(
        headers[header::CONTENT_TYPE],
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    );
    assert_eq!(
        headers[header::CONTENT_DISPOSITION],
        "attachment; filename=\"kasl-team-timesheet-2026-08.xlsx\""
    );
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");

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
    assert!(workbook.contains(r#"name="Timesheet""#), "{workbook}");
    assert!(workbook.contains(r#"name="Reports""#), "{workbook}");

    let strings = part("xl/sharedStrings.xml");
    for expected in [
        "Timesheet - August 2026",
        "Reports - August 2026",
        "inside",
        "Engineering",
        "Calendar: 31 Aug holiday (Summer holiday).",
        // The day away, the date owed and unrecorded, the open day.
        "V",
        "?",
        "…",
        // On time, late, missing, pending.
        "✓",
        "L",
        "✗",
        "·",
    ] {
        assert!(strings.contains(expected), "{expected} should be in the workbook: {strings}");
    }
    assert!(!strings.contains("outside"), "nobody the manager may not see: {strings}");

    // Hours are numbers a spreadsheet can sum: Monday's 8.5 is a value.
    let grid = part("xl/worksheets/sheet1.xml");
    assert!(grid.contains("<v>8.5</v>"), "{grid}");

    let (status, entries) = server.get_with_cookie("/api/v1/audit", team.admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{entries}");
    let exported: Vec<&Value> = entries
        .as_array()
        .expect("entries")
        .iter()
        .filter(|entry| entry["action"] == "hours.exported")
        .collect();
    assert_eq!(exported.len(), 1, "{entries}");
    assert_eq!(exported[0]["details"]["file"], "timesheet");
    assert_eq!(exported[0]["details"]["people"], 2);

    // Your own month is nobody else's record.
    let (status, headers, _) = server
        .get_file_with_cookie("/api/v1/me/timesheet.xlsx?month=2026-08", team.employee.as_deref())
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_DISPOSITION], "attachment; filename=\"kasl-me-timesheet-2026-08.xlsx\"");
    assert_eq!(server.scalar::<i64>("SELECT count(*) FROM audit_log WHERE action = 'hours.exported'").await, 1);

    server.close().await;
}
