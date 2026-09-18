//! The production calendar and the norm, through the real router.
//!
//! The unit tests in `calendar` cover the arithmetic against a calendar held
//! in memory. These cover what only a database and the HTTP surface can say:
//! that a year survives a round trip, that only an administrator may write
//! one, that the norm on `/me/days` reflects the rows actually stored, and
//! that a day of leave owes nothing.

mod support;

use axum::http::StatusCode;
use serde_json::json;
use support::TestServer;

const ADMIN: (&str, &str) = ("admin@example.test", "admin-password");
const EMPLOYEE: (&str, &str) = ("employee@example.test", "employee-password");

/// A week in September 2026: Monday the 14th to Sunday the 20th.
const MONDAY: &str = "2026-09-14";
const SUNDAY: &str = "2026-09-20";

/// Eight hours, in seconds - a full day at the default norm.
const FULL_DAY: i64 = 8 * 3600;

/// Signs in as the seeded administrator.
async fn admin_cookie(server: &TestServer) -> String {
    server.add_admin(ADMIN.0, ADMIN.1).await;
    let (status, cookie, _) = server.login(ADMIN.0, ADMIN.1).await;
    assert_eq!(status, StatusCode::OK, "the administrator should sign in");
    cookie.expect("a successful login sets a cookie")
}

/// Signs in as the employee whose agent the fixture provisions.
async fn employee_cookie(server: &TestServer) -> String {
    server.set_password(EMPLOYEE.0, EMPLOYEE.1).await;
    let (status, cookie, _) = server.login(EMPLOYEE.0, EMPLOYEE.1).await;
    assert_eq!(status, StatusCode::OK, "the employee should sign in");
    cookie.expect("a successful login sets a cookie")
}

/// A day the agent would send, with an optional kind.
fn day(date: &str, hours: i64, kind: Option<&str>) -> serde_json::Value {
    let mut day = json!({
        "date": date,
        "started_at": format!("{date}T09:00:00+00:00"),
        "ended_at": format!("{date}T{:02}:00:00+00:00", 9 + hours),
        "pauses": [],
        "tasks": [],
    });
    if let Some(kind) = kind {
        day["kind"] = json!(kind);
    }
    day
}

#[tokio::test]
async fn a_year_round_trips_through_the_api() {
    let Some(server) = TestServer::start().await else { return };
    let cookie = admin_cookie(&server).await;

    let (status, _, body) = server
        .put_with_cookie(
            "/api/v1/calendar?year=2026",
            Some(&cookie),
            json!({
                "days": [
                    {"date": "2026-01-01", "kind": "holiday", "note": "New Year"},
                    {"date": "2026-12-31", "kind": "short_day"},
                    {"date": "2026-11-07", "kind": "working_weekend", "note": "transferred"},
                ]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["days"].as_array().unwrap().len(), 3);
    // The full day comes back with the calendar, so a screen showing the year
    // knows what a day is worth without a second request.
    assert_eq!(body["standard_hours"], 8.0);

    let (status, body) = server.get_with_cookie("/api/v1/calendar?year=2026", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    let days = body["days"].as_array().unwrap();
    assert_eq!(days.len(), 3, "the year is what was written: {body}");
    // Ordered by date, which is what a screen listing a year renders directly.
    assert_eq!(days[0]["date"], "2026-01-01");
    assert_eq!(days[0]["kind"], "holiday");
    assert_eq!(days[0]["note"], "New Year");
    assert_eq!(days[1]["date"], "2026-11-07");
    assert_eq!(days[1]["kind"], "working_weekend");
    assert_eq!(days[2]["kind"], "short_day");
    assert!(days[2]["note"].is_null(), "a day without a name says so");

    server.close().await;
}

#[tokio::test]
async fn writing_a_year_replaces_it_rather_than_adding_to_it() {
    // The correction case: a calendar that was entered wrong is replaced by
    // the document that is right. A merge would leave the wrong rows in place
    // with nothing on the screen to say they are still there (ADR 0017).
    let Some(server) = TestServer::start().await else { return };
    let cookie = admin_cookie(&server).await;

    let (status, _, _) = server
        .put_with_cookie(
            "/api/v1/calendar?year=2026",
            Some(&cookie),
            json!({ "days": [{"date": "2026-05-01", "kind": "holiday"}, {"date": "2026-05-09", "kind": "holiday"}] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, body) = server
        .put_with_cookie(
            "/api/v1/calendar?year=2026",
            Some(&cookie),
            json!({ "days": [{"date": "2026-05-01", "kind": "holiday"}] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let days = body["days"].as_array().unwrap();
    assert_eq!(days.len(), 1, "the second write is the whole year: {body}");
    assert_eq!(days[0]["date"], "2026-05-01");

    server.close().await;
}

#[tokio::test]
async fn one_year_does_not_touch_another() {
    // A replacement is scoped to the year it was asked for. Without the
    // bound, entering next year's calendar would silently wipe this one's.
    let Some(server) = TestServer::start().await else { return };
    let cookie = admin_cookie(&server).await;

    let (status, _, _) = server
        .put_with_cookie(
            "/api/v1/calendar?year=2026",
            Some(&cookie),
            json!({ "days": [{"date": "2026-01-01", "kind": "holiday"}] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, _) = server
        .put_with_cookie(
            "/api/v1/calendar?year=2027",
            Some(&cookie),
            json!({ "days": [{"date": "2027-01-01", "kind": "holiday"}] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = server.get_with_cookie("/api/v1/calendar?year=2026", Some(&cookie)).await;
    assert_eq!(body["days"].as_array().unwrap().len(), 1, "2026 survived writing 2027: {body}");

    server.close().await;
}

#[tokio::test]
async fn a_date_outside_the_year_is_refused_before_anything_is_written() {
    let Some(server) = TestServer::start().await else { return };
    let cookie = admin_cookie(&server).await;

    // A year that is already entered, so the refusal has something to destroy
    // if it were not checked before the delete.
    let (status, _, _) = server
        .put_with_cookie(
            "/api/v1/calendar?year=2026",
            Some(&cookie),
            json!({ "days": [{"date": "2026-01-01", "kind": "holiday"}] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, body) = server
        .put_with_cookie(
            "/api/v1/calendar?year=2026",
            Some(&cookie),
            json!({ "days": [{"date": "2026-03-08", "kind": "holiday"}, {"date": "2027-01-01", "kind": "holiday"}] }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("days[1]"), "the message names the day: {body}");

    let (_, body) = server.get_with_cookie("/api/v1/calendar?year=2026", Some(&cookie)).await;
    assert_eq!(body["days"].as_array().unwrap().len(), 1, "the refused write changed nothing: {body}");
    assert_eq!(body["days"][0]["date"], "2026-01-01");

    server.close().await;
}

#[tokio::test]
async fn a_duplicate_date_is_the_administrators_typo_not_a_server_fault() {
    let Some(server) = TestServer::start().await else { return };
    let cookie = admin_cookie(&server).await;

    let (status, _, body) = server
        .put_with_cookie(
            "/api/v1/calendar?year=2026",
            Some(&cookie),
            json!({ "days": [{"date": "2026-01-01", "kind": "holiday"}, {"date": "2026-01-01", "kind": "short_day"}] }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("2026-01-01"), "{body}");

    server.close().await;
}

#[tokio::test]
async fn an_employee_reads_the_calendar_but_cannot_write_it() {
    // Which days of the year are worked is not a secret from the people
    // working them - but the calendar every norm is computed from is not
    // theirs to change.
    let Some(server) = TestServer::start().await else { return };
    let admin = admin_cookie(&server).await;
    let employee = employee_cookie(&server).await;

    let (status, _, _) = server
        .put_with_cookie(
            "/api/v1/calendar?year=2026",
            Some(&admin),
            json!({ "days": [{"date": "2026-01-01", "kind": "holiday"}] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = server.get_with_cookie("/api/v1/calendar?year=2026", Some(&employee)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["days"].as_array().unwrap().len(), 1);

    let (status, _, _) = server
        .put_with_cookie(
            "/api/v1/calendar?year=2026",
            Some(&employee),
            json!({ "days": [{"date": "2026-01-02", "kind": "holiday"}] }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "an employee cannot rewrite the year");

    let (status, _) = server.get_with_cookie("/api/v1/calendar?year=2026", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "and a stranger reads nothing");

    server.close().await;
}

#[tokio::test]
async fn the_full_day_and_a_share_of_it_are_recorded_when_changed() {
    let Some(server) = TestServer::start().await else { return };
    let cookie = admin_cookie(&server).await;

    let (status, _, body) = server
        .put_with_cookie("/api/v1/calendar/standard-hours", Some(&cookie), json!({ "standard_hours": 7.5 }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["standard_hours"], 7.5);

    // Zero is not a working day, and neither is a typo of ten hours entered
    // as a share. Both are refused rather than quietly applied to everyone.
    let (status, _, _) = server
        .put_with_cookie("/api/v1/calendar/standard-hours", Some(&cookie), json!({ "standard_hours": 0 }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let employee: String = sqlx::query_scalar("SELECT id::text FROM users WHERE lower(email) = lower($1)")
        .bind(EMPLOYEE.0)
        .fetch_one(&server.pool)
        .await
        .expect("the seeded employee exists");

    let (status, _, body) = server
        .put_with_cookie(&format!("/api/v1/users/{employee}/work-rate"), Some(&cookie), json!({ "work_rate": 0.5 }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["work_rate"], 0.5);

    let (status, _, _) = server
        .put_with_cookie(&format!("/api/v1/users/{employee}/work-rate"), Some(&cookie), json!({ "work_rate": 10 }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "ten days in one is a typo, not a rate");

    // Both are figures every screen is computed from, so both leave a trace
    // naming who moved them.
    let (status, body) = server.get_with_cookie("/api/v1/audit", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    let actions: Vec<&str> = body
        .as_array()
        .expect("the audit log answers a list")
        .iter()
        .map(|entry| entry["action"].as_str().unwrap_or_default())
        .collect();
    assert!(actions.contains(&"calendar.standard_hours_changed"), "{actions:?}");
    assert!(actions.contains(&"calendar.work_rate_changed"), "{actions:?}");

    server.close().await;
}

#[tokio::test]
async fn a_week_owes_five_days_and_a_holiday_takes_one_away() {
    let Some(server) = TestServer::start().await else { return };
    let admin = admin_cookie(&server).await;
    let employee = employee_cookie(&server).await;

    let (status, body) = server
        .get_with_cookie(&format!("/api/v1/me/days?from={MONDAY}&to={SUNDAY}"), Some(&employee))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["progress"]["norm_seconds"].as_i64().unwrap(),
        5 * FULL_DAY,
        "an empty calendar works the five weekdays: {body}"
    );
    assert_eq!(body["progress"]["standard_hours"], 8.0);
    assert_eq!(body["progress"]["work_rate"], 1.0);

    // Wednesday becomes a holiday.
    let (status, _, _) = server
        .put_with_cookie(
            "/api/v1/calendar?year=2026",
            Some(&admin),
            json!({ "days": [{"date": "2026-09-16", "kind": "holiday", "note": "a holiday"}] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = server
        .get_with_cookie(&format!("/api/v1/me/days?from={MONDAY}&to={SUNDAY}"), Some(&employee))
        .await;
    assert_eq!(
        body["progress"]["norm_seconds"].as_i64().unwrap(),
        4 * FULL_DAY,
        "the week owes four days once a Wednesday is a holiday: {body}"
    );

    server.close().await;
}

#[tokio::test]
async fn leave_is_excused_rather_than_reported_as_hours_missing() {
    // The defect this exists to prevent: a fortnight of holiday reported as
    // eighty hours missing, which is the most alarming possible way to be
    // wrong about somebody on a beach.
    let Some(server) = TestServer::start().await else { return };
    let employee = employee_cookie(&server).await;

    // Monday worked, Tuesday and Wednesday on leave, sent by the agent.
    let (status, body) = server.post_day(&server.token, day(MONDAY, 8, None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["kind"], "work", "a day that says nothing is a worked day");

    for date in ["2026-09-15", "2026-09-16"] {
        let (status, body) = server.post_day(&server.token, day(date, 0, Some("vacation"))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["kind"], "vacation", "the server echoes the kind it stored");
    }

    let (status, body) = server
        .get_with_cookie(&format!("/api/v1/me/days?from={MONDAY}&to={SUNDAY}"), Some(&employee))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["progress"]["norm_seconds"].as_i64().unwrap(),
        3 * FULL_DAY,
        "two days of leave are not owed: {body}"
    );

    let days = body["days"].as_array().unwrap();
    assert_eq!(days.len(), 3);
    assert_eq!(days[0]["kind"], "work");
    assert_eq!(days[0]["norm_seconds"].as_i64().unwrap(), FULL_DAY);
    assert_eq!(days[1]["kind"], "vacation");
    assert_eq!(days[1]["norm_seconds"].as_i64().unwrap(), 0, "a day away owes nothing");

    server.close().await;
}

#[tokio::test]
async fn a_half_time_person_owes_half_the_week() {
    let Some(server) = TestServer::start().await else { return };
    let admin = admin_cookie(&server).await;
    let employee = employee_cookie(&server).await;

    let id: String = sqlx::query_scalar("SELECT id::text FROM users WHERE lower(email) = lower($1)")
        .bind(EMPLOYEE.0)
        .fetch_one(&server.pool)
        .await
        .expect("the seeded employee exists");

    let (status, _, _) = server
        .put_with_cookie(&format!("/api/v1/users/{id}/work-rate"), Some(&admin), json!({ "work_rate": 0.5 }))
        .await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = server
        .get_with_cookie(&format!("/api/v1/me/days?from={MONDAY}&to={SUNDAY}"), Some(&employee))
        .await;
    assert_eq!(
        body["progress"]["norm_seconds"].as_i64().unwrap(),
        5 * FULL_DAY / 2,
        "half a full day, five days over: {body}"
    );
    assert_eq!(body["progress"]["work_rate"], 0.5, "the rate is carried so the screen can say why");

    server.close().await;
}

#[tokio::test]
async fn the_team_table_carries_a_norm_per_person() {
    let Some(server) = TestServer::start().await else { return };
    let admin = admin_cookie(&server).await;

    let id: String = sqlx::query_scalar("SELECT id::text FROM users WHERE lower(email) = lower($1)")
        .bind(EMPLOYEE.0)
        .fetch_one(&server.pool)
        .await
        .expect("the seeded employee exists");

    // Half time, one day of the week on leave, and a working Saturday: all
    // three have to land in one figure.
    let (status, _, _) = server
        .put_with_cookie(&format!("/api/v1/users/{id}/work-rate"), Some(&admin), json!({ "work_rate": 0.5 }))
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, _) = server
        .put_with_cookie(
            "/api/v1/calendar?year=2026",
            Some(&admin),
            json!({ "days": [{"date": "2026-09-19", "kind": "working_weekend", "note": "transferred"}] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = server.post_day(&server.token, day("2026-09-15", 0, Some("sick"))).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = server
        .get_with_cookie(&format!("/api/v1/team/days?from={MONDAY}&to={SUNDAY}"), Some(&admin))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["standard_hours"], 8.0);

    let member = body["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|member| member["email"] == EMPLOYEE.0)
        .expect("the employee is in the table");

    // Five weekdays plus the transferred Saturday, less the day off sick, at
    // half rate: five half-days.
    //
    // The figure is checked against a week without the Saturday below rather
    // than taken on its own: five weekdays minus a sick day plus a working
    // Saturday comes to the same number as five weekdays minus nothing, so
    // this assertion alone is satisfied by a server that ignores both. That
    // is the shape of green that says nothing (the same tie-break that hid a
    // missing rule in kilna).
    assert_eq!(member["norm_seconds"].as_i64().unwrap(), 5 * FULL_DAY / 2, "{member}");
    assert_eq!(member["days_away"].as_i64().unwrap(), 1);
    assert_eq!(member["work_rate"], 0.5);

    // The week before holds neither the Saturday nor the sick day: four and a
    // half fewer half-days than the week above would have owed without them.
    let (_, body) = server.get_with_cookie("/api/v1/team/days?from=2026-09-07&to=2026-09-13", Some(&admin)).await;
    let plain = body["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|member| member["email"] == EMPLOYEE.0)
        .expect("the employee is in the table");
    assert_eq!(
        plain["norm_seconds"].as_i64().unwrap(),
        5 * FULL_DAY / 2,
        "an ordinary week is five half-days: {plain}"
    );
    assert_eq!(plain["days_away"].as_i64().unwrap(), 0);

    // And the Saturday on its own is a working day: a weekend the calendar
    // moved into the week owes its hours, which a list of holidays could
    // never say (ADR 0017).
    let (_, body) = server.get_with_cookie("/api/v1/team/days?from=2026-09-19&to=2026-09-20", Some(&admin)).await;
    let weekend = body["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|member| member["email"] == EMPLOYEE.0)
        .expect("the employee is in the table");
    assert_eq!(
        weekend["norm_seconds"].as_i64().unwrap(),
        FULL_DAY / 2,
        "the transferred Saturday owes half a day, the Sunday nothing: {weekend}"
    );

    server.close().await;
}

#[tokio::test]
async fn an_agent_that_never_heard_of_a_kind_still_uploads_a_worked_day() {
    // The compatibility hinge, driven through the real route: every kasl
    // shipped before v1.35 sends no `kind`, and the installed base must not
    // land on permanent leave.
    let Some(server) = TestServer::start().await else { return };

    let (status, body) = server.post_day(&server.token, day(MONDAY, 8, None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["kind"], "work");

    let stored: String = sqlx::query_scalar("SELECT kind::text FROM workdays WHERE date = $1")
        .bind(MONDAY.parse::<chrono::NaiveDate>().unwrap())
        .fetch_one(&server.pool)
        .await
        .expect("the day was stored");
    assert_eq!(stored, "work");

    server.close().await;
}

#[tokio::test]
async fn a_day_that_stops_being_leave_owes_its_hours_again() {
    // The correction path: the employee marked a day as holiday, then took it
    // back in kasl. A re-upload has to move the norm with it - otherwise a day
    // excused once stays excused forever, and the week is quietly short.
    let Some(server) = TestServer::start().await else { return };
    let employee = employee_cookie(&server).await;

    let (status, _) = server.post_day(&server.token, day(MONDAY, 8, Some("vacation"))).await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = server
        .get_with_cookie(&format!("/api/v1/me/days?from={MONDAY}&to={SUNDAY}"), Some(&employee))
        .await;
    assert_eq!(body["progress"]["norm_seconds"].as_i64().unwrap(), 4 * FULL_DAY);

    let (status, body) = server.post_day(&server.token, day(MONDAY, 8, Some("work"))).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (_, body) = server
        .get_with_cookie(&format!("/api/v1/me/days?from={MONDAY}&to={SUNDAY}"), Some(&employee))
        .await;
    assert_eq!(
        body["progress"]["norm_seconds"].as_i64().unwrap(),
        5 * FULL_DAY,
        "the day owes its hours again: {body}"
    );

    server.close().await;
}
