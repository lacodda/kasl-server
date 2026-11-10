//! Accruals through the real router and a real database (ADR 0028).
//!
//! The arithmetic is unit-tested in `pay::accrual`, where a day is a struct.
//! These hold what only the HTTP surface can: that a date read alone earns
//! what it earns inside its week - the handler reads whole weeks - that the
//! figure priced is the one every other screen reads for the day, that the
//! overtime rules survive the trip into the columns and back, and who may read
//! the money.

mod support;

use axum::http::StatusCode;
use serde_json::{Value, json};
use support::TestServer;

const ADMIN: (&str, &str) = ("admin@example.test", "admin-password");
const EMPLOYEE: (&str, &str) = ("employee@example.test", "employee-password");

/// The week every test works in: Monday 2026-10-05 to Sunday 2026-10-11, in
/// the past whatever day the suite runs, with no calendar entered.
const WEEK: [&str; 5] = ["2026-10-05", "2026-10-06", "2026-10-07", "2026-10-08", "2026-10-09"];

async fn admin_and_employee(server: &TestServer) -> (String, String) {
    server.add_admin(ADMIN.0, ADMIN.1).await;
    let (status, cookie, _) = server.login(ADMIN.0, ADMIN.1).await;
    assert_eq!(status, StatusCode::OK, "the administrator should sign in");
    let employee: String = sqlx::query_scalar("SELECT id::text FROM users WHERE lower(email) = lower($1)")
        .bind(EMPLOYEE.0)
        .fetch_one(&server.pool)
        .await
        .expect("the seeded employee exists");
    (cookie.expect("a successful login sets a cookie"), employee)
}

async fn employee_cookie(server: &TestServer) -> String {
    server.set_password(EMPLOYEE.0, EMPLOYEE.1).await;
    let (status, cookie, _) = server.login(EMPLOYEE.0, EMPLOYEE.1).await;
    assert_eq!(status, StatusCode::OK, "the employee should sign in");
    cookie.expect("a successful login sets a cookie")
}

/// A finished day from nine, `hours` long, with `paused` minutes of lunch.
fn day(date: &str, hours: u32, paused: u32) -> Value {
    let mut pauses = Vec::new();
    if paused > 0 {
        pauses.push(json!({
            "started_at": format!("{date}T12:00:00+00:00"),
            "ended_at": format!("{date}T12:{paused:02}:00+00:00"),
            "duration_seconds": paused * 60,
        }));
    }
    json!({
        "date": date,
        "started_at": format!("{date}T09:00:00+00:00"),
        "ended_at": format!("{date}T{:02}:{paused:02}:00+00:00", 9 + hours),
        "pauses": pauses,
        "tasks": []
    })
}

async fn post_week(server: &TestServer, hours: u32) {
    for date in WEEK {
        let (status, body) = server.post_day(&server.token, day(date, hours, 30)).await;
        assert!(status.is_success(), "{date}: {body}");
    }
}

async fn record(server: &TestServer, cookie: &str, user: &str, version: Value) -> Value {
    let (status, _, body) = server.post_with_cookie(&format!("/api/v1/users/{user}/pay-terms"), Some(cookie), version).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body
}

async fn accruals(server: &TestServer, cookie: &str, from: &str, to: &str) -> Value {
    let (status, body) = server.get_with_cookie(&format!("/api/v1/me/accruals?from={from}&to={to}"), Some(cookie)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// A line as (kind, rule, seconds, multiplier, amount).
type LineRow = (String, Option<String>, Option<i64>, String, String);

/// A day's lines, for asserting them whole.
fn lines(day: &Value) -> Vec<LineRow> {
    day["lines"]
        .as_array()
        .expect("a day lists its lines")
        .iter()
        .map(|line| {
            (
                line["kind"].as_str().unwrap().to_string(),
                line["rule"].as_str().map(str::to_string),
                line["seconds"].as_i64(),
                line["multiplier"].as_str().unwrap().to_string(),
                line["amount"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn line(kind: &str, rule: Option<&str>, seconds: i64, multiplier: &str, amount: &str) -> LineRow {
    (
        kind.to_string(),
        rule.map(str::to_string),
        Some(seconds),
        multiplier.to_string(),
        amount.to_string(),
    )
}

#[tokio::test]
async fn a_long_week_accrues_its_overtime_and_a_date_alone_earns_the_same() {
    let Some(server) = TestServer::start().await else { return };
    let (admin, employee) = admin_and_employee(&server).await;
    record(
        &server,
        &admin,
        &employee,
        json!({
            "effective_from": "2026-01-01", "basis": "hourly", "rate": "20", "currency": "USD",
            "overtime": { "weekly": { "after": "40", "multiplier": "1.5" } }
        }),
    )
    .await;
    // Nine and a half hours with half an hour of lunch: nine worked a day,
    // forty-five in the week.
    post_week(&server, 9).await;

    let cookie = employee_cookie(&server).await;
    let week = accruals(&server, &cookie, "2026-10-05", "2026-10-11").await;
    let days = week["days"].as_array().unwrap();
    assert_eq!(days.len(), 5, "{week}");
    assert_eq!(days[0]["worked_seconds"], 9 * 3600, "the figure every screen reads: the lunch is not paid");
    assert_eq!(lines(&days[0]), [line("hours", None, 9 * 3600, "1", "180.00")]);
    assert_eq!(
        lines(&days[4]),
        [
            line("hours", None, 4 * 3600, "1", "80.00"),
            line("overtime", Some("weekly"), 5 * 3600, "1.5", "150.00"),
        ],
        "Friday passes forty after four hours: {}",
        days[4]
    );
    assert_eq!(days[4]["status"], "accrued");
    assert_eq!(days[4]["terms"]["currency"], "USD");
    assert_eq!(
        week["totals"],
        json!([{ "currency": "USD", "amount": "950.00", "worked_seconds": 45 * 3600, "overtime_seconds": 5 * 3600 }])
    );

    // Friday alone: the handler reads Friday's whole week, so the weekly
    // threshold is reached exactly where it was.
    let friday = accruals(&server, &cookie, "2026-10-09", "2026-10-09").await;
    let alone = &friday["days"].as_array().unwrap()[0];
    assert_eq!(alone, &days[4], "a date earns the same whichever range asks");
    assert_eq!(friday["totals"][0]["amount"], "230.00");

    server.close().await;
}

#[tokio::test]
async fn the_rules_go_into_the_columns_and_come_back() {
    let Some(server) = TestServer::start().await else { return };
    let (admin, employee) = admin_and_employee(&server).await;
    let rules = json!({
        "daily": { "after": "norm", "multiplier": "1.5", "then": { "after": "2", "multiplier": "2" } },
        "weekly": { "after": "40", "multiplier": "1.25", "then": null },
        "rest_days": "2"
    });
    let history = record(
        &server,
        &admin,
        &employee,
        json!({ "effective_from": "2026-01-01", "basis": "daily", "rate": "200", "currency": "EUR", "overtime": rules }),
    )
    .await;
    assert_eq!(history["versions"][0]["overtime"], rules, "{history}");

    // A version without rules answers each of them as absent.
    let history = record(
        &server,
        &admin,
        &employee,
        json!({ "effective_from": "2026-06-01", "basis": "hourly", "rate": "25", "currency": "EUR" }),
    )
    .await;
    assert_eq!(history["versions"][0]["overtime"], json!({ "daily": null, "weekly": null, "rest_days": null }));

    // What the API refuses, the database refuses too - for a writer that is
    // not the API.
    let id: i64 = sqlx::query_scalar("SELECT id FROM pay_terms ORDER BY id LIMIT 1")
        .fetch_one(&server.pool)
        .await
        .unwrap();
    let user: uuid::Uuid = sqlx::query_scalar("SELECT user_id FROM pay_terms WHERE id = $1")
        .bind(id)
        .fetch_one(&server.pool)
        .await
        .unwrap();
    for (columns, why) in [
        ("daily_multiplier, daily_then_after_hours, daily_then_multiplier", "1.5, 2, 1.25"),
        ("daily_after_hours", "8"),
        ("weekly_multiplier", "1"),
        (
            "daily_after_hours, daily_multiplier, daily_then_after_hours, daily_then_multiplier",
            "20, 1.5, 5, 2",
        ),
    ] {
        let sql = format!(
            "INSERT INTO pay_terms (user_id, effective_from, basis, rate, currency, recorded_by, {columns})
             VALUES ($1, '2026-02-01', 'hourly', 10, 'EUR', $1, {why})"
        );
        let refused = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(user).execute(&server.pool).await;
        assert!(refused.is_err(), "the database accepted {columns} = {why}");
    }
    // A rule cannot be added to a version after the fact: it is the row.
    let update = sqlx::query("UPDATE pay_terms SET daily_multiplier = 1.5 WHERE id = $1")
        .bind(id)
        .execute(&server.pool)
        .await;
    assert!(update.is_err(), "a version's rules are as fixed as its rate");

    server.close().await;
}

#[tokio::test]
async fn a_rule_the_person_could_not_mean_is_refused() {
    let Some(server) = TestServer::start().await else { return };
    let (admin, employee) = admin_and_employee(&server).await;
    let path = format!("/api/v1/users/{employee}/pay-terms");
    let with = |overtime: Value| json!({ "effective_from": "2026-01-01", "basis": "hourly", "rate": "10", "currency": "USD", "overtime": overtime });

    for (overtime, why) in [
        (json!({ "daily": { "after": "8", "multiplier": "1" } }), "a multiplier that pays no more"),
        (json!({ "daily": { "after": "25", "multiplier": "1.5" } }), "a day of 25 hours"),
        (
            json!({ "daily": { "after": "8", "multiplier": "2", "then": { "after": "2", "multiplier": "1.5" } } }),
            "a ladder that goes down",
        ),
        (json!({ "weekly": { "after": "forty", "multiplier": "1.5" } }), "hours that are not a number"),
        (json!({ "rest_days": "0.5" }), "a rest day that pays less"),
    ] {
        let (status, _, body) = server.post_with_cookie(&path, Some(&admin), with(overtime)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{why}: {body}");
    }
    let (status, _, body) = server
        .post_with_cookie(&path, Some(&admin), with(json!({ "dayly": { "after": "8", "multiplier": "1.5" } })))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "a misspelt rule does not vanish: {body}");
    assert_eq!(server.count("pay_terms").await, 0, "nothing refused was written");

    server.close().await;
}

#[tokio::test]
async fn what_is_not_accrued_is_said() {
    let Some(server) = TestServer::start().await else { return };
    let (admin, employee) = admin_and_employee(&server).await;
    post_week(&server, 8).await;
    // Terms from Wednesday only; a salary from Friday.
    record(
        &server,
        &admin,
        &employee,
        json!({ "effective_from": "2026-10-07", "basis": "hourly", "rate": "10", "currency": "USD" }),
    )
    .await;
    record(
        &server,
        &admin,
        &employee,
        json!({ "effective_from": "2026-10-09", "basis": "monthly", "rate": "4000", "currency": "USD" }),
    )
    .await;

    let cookie = employee_cookie(&server).await;
    let week = accruals(&server, &cookie, "2026-10-05", "2026-10-11").await;
    let statuses: Vec<&str> = week["days"].as_array().unwrap().iter().map(|day| day["status"].as_str().unwrap()).collect();
    assert_eq!(statuses, ["no_terms", "no_terms", "accrued", "accrued", "salaried"], "{week}");
    assert_eq!(week["days"][0]["amount"], Value::Null, "no terms is not a rate of zero");
    assert_eq!(
        week["not_accrued"],
        json!({ "no_terms_seconds": 2 * 8 * 3600, "salaried_seconds": 8 * 3600, "open_days": 0 })
    );
    assert_eq!(week["totals"][0]["amount"], "160.00");

    server.close().await;
}

#[tokio::test]
async fn the_money_is_read_by_the_person_and_administrators_only() {
    let Some(server) = TestServer::start().await else { return };
    let (admin, employee) = admin_and_employee(&server).await;
    record(
        &server,
        &admin,
        &employee,
        json!({ "effective_from": "2026-01-01", "basis": "hourly", "rate": "10", "currency": "USD" }),
    )
    .await;
    post_week(&server, 8).await;
    let path = format!("/api/v1/users/{employee}/accruals?from=2026-10-05&to=2026-10-11");

    // A manager of the employee's own department sees the days, not the money.
    server.add_agent("lead@example.test", "lead-token").await;
    server.set_password("lead@example.test", "correct horse").await;
    let lead: String = sqlx::query_scalar("SELECT id::text FROM users WHERE email = 'lead@example.test'")
        .fetch_one(&server.pool)
        .await
        .unwrap();
    let (status, _, _) = server
        .patch_with_cookie(&format!("/api/v1/users/{lead}"), Some(&admin), json!({ "role": "manager" }))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, _, body) = server
        .post_with_cookie("/api/v1/departments", Some(&admin), json!({ "name": "Engineering", "manager_id": lead }))
        .await;
    let department = body["id"].as_str().unwrap().to_string();
    server
        .put_with_cookie(
            &format!("/api/v1/users/{employee}/department"),
            Some(&admin),
            json!({ "department_id": department }),
        )
        .await;
    let (_, manager, _) = server.login("lead@example.test", "correct horse").await;
    let manager = manager.expect("the manager signs in");
    let (status, _) = server.get_with_cookie(&path, Some(&manager)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a manager does not read the pay");

    let cookie = employee_cookie(&server).await;
    let (status, _) = server.get_with_cookie(&path, Some(&cookie)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "the person reads their own at /me");

    let (status, _) = server.get_with_cookie("/api/v1/me/accruals?from=2026-10-05&to=2026-10-11", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, body) = server.get_with_cookie(&path, Some(&admin)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["totals"][0]["amount"], "400.00");
    let mine = accruals(&server, &cookie, "2026-10-05", "2026-10-11").await;
    assert_eq!(body, mine, "the administrator reads what the person reads");

    // Recorded, with the range and never an amount.
    let (status, entries) = server.get_with_cookie("/api/v1/audit?action=accruals.viewed", Some(&admin)).await;
    assert_eq!(status, StatusCode::OK, "{entries}");
    let entries = entries.as_array().unwrap();
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0]["details"], json!({ "from": "2026-10-05", "to": "2026-10-11" }));

    let (status, _) = server
        .get_with_cookie(
            "/api/v1/users/00000000-0000-0000-0000-000000000000/accruals?from=2026-10-05&to=2026-10-11",
            Some(&admin),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = server.get_with_cookie("/api/v1/me/accruals?from=2026-10-11&to=2026-10-05", Some(&cookie)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "a range that ends before it starts");

    server.close().await;
}
