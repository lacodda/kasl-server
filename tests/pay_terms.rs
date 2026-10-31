//! Pay terms through the real router and a real database (ADR 0027).
//!
//! The unit tests in `pay::terms` hold the rule of which version applies to a
//! date. These hold what only the HTTP surface and the schema can say: who may
//! read and write terms, that the history only grows - in the database itself,
//! not only in the handler - and that the audit log learns of every write and
//! every read of somebody else's pay without learning the amount.

mod support;

use axum::http::StatusCode;
use serde_json::{Value, json};
use support::TestServer;

const ADMIN: (&str, &str) = ("admin@example.test", "admin-password");
const EMPLOYEE: (&str, &str) = ("employee@example.test", "employee-password");

/// The administrator's cookie and the seeded employee's id.
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

/// Records a version and answers the history that comes back.
async fn record(server: &TestServer, cookie: &str, user: &str, version: Value) -> Value {
    let (status, _, body) = server.post_with_cookie(&format!("/api/v1/users/{user}/pay-terms"), Some(cookie), version).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body
}

fn hourly(effective_from: &str, rate: &str) -> Value {
    json!({ "effective_from": effective_from, "basis": "hourly", "rate": rate, "currency": "USD" })
}

/// The versions of a history as (start, rate, state), in the order answered.
fn rows(history: &Value) -> Vec<(String, String, String)> {
    history["versions"]
        .as_array()
        .expect("a history lists its versions")
        .iter()
        .map(|version| {
            (
                version["effective_from"].as_str().unwrap().to_string(),
                version["rate"].as_str().unwrap().to_string(),
                version["state"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// The audit entries with one action, newest first.
async fn audit(server: &TestServer, cookie: &str, action: &str) -> Vec<Value> {
    let (status, body) = server.get_with_cookie(&format!("/api/v1/audit?action={action}"), Some(cookie)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body.as_array().expect("the audit log answers a list").clone()
}

fn row(start: &str, rate: &str, state: &str) -> (String, String, String) {
    (start.to_string(), rate.to_string(), state.to_string())
}

#[tokio::test]
async fn a_raise_is_a_new_version_and_the_old_one_stays() {
    let Some(server) = TestServer::start().await else { return };
    let (admin, employee) = admin_and_employee(&server).await;

    let history = record(&server, &admin, &employee, hourly("2026-01-01", "20")).await;
    assert_eq!(rows(&history), [row("2026-01-01", "20.00", "current")], "{history}");
    let first = &history["versions"][0];
    assert_eq!(first["basis"], "hourly");
    assert_eq!(first["currency"], "USD");
    assert_eq!(first["recorded_by"]["name"], ADMIN.0, "{first}");

    // The server's today is real, so the raise is dated safely in the past and
    // the next one safely ahead of it.
    record(&server, &admin, &employee, hourly("2026-03-01", "25.5")).await;
    let history = record(&server, &admin, &employee, hourly("2099-01-01", "30")).await;
    assert_eq!(
        rows(&history),
        [
            row("2099-01-01", "30.00", "upcoming"),
            row("2026-03-01", "25.50", "current"),
            row("2026-01-01", "20.00", "ended"),
        ],
        "{history}"
    );

    // The person reads the same history - their own, every version of it.
    let cookie = employee_cookie(&server).await;
    let (status, mine) = server.get_with_cookie("/api/v1/me/pay-terms", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{mine}");
    assert_eq!(mine["versions"], history["versions"]);
    assert_eq!(mine["user_id"], employee.as_str());

    server.close().await;
}

#[tokio::test]
async fn a_mistake_is_corrected_by_a_later_version_with_the_same_start() {
    let Some(server) = TestServer::start().await else { return };
    let (admin, employee) = admin_and_employee(&server).await;

    record(&server, &admin, &employee, hourly("2026-01-01", "20")).await;
    let typo = record(&server, &admin, &employee, hourly("2026-03-01", "2500")).await;
    let typo_id = typo["versions"][0]["id"].as_i64().unwrap();
    let fixed = record(
        &server,
        &admin,
        &employee,
        json!({ "effective_from": "2026-03-01", "basis": "hourly", "rate": "25", "currency": "USD", "note": "typo in the March raise" }),
    )
    .await;

    assert_eq!(
        rows(&fixed),
        [
            row("2026-03-01", "25.00", "current"),
            row("2026-03-01", "2500.00", "superseded"),
            row("2026-01-01", "20.00", "ended"),
        ],
        "the correction takes the date over and the mistake stays readable: {fixed}"
    );
    assert_eq!(fixed["versions"][0]["note"], "typo in the March raise");

    // The log names what was corrected, and never the amount: the typo is a
    // salary-sized figure an administrator reading the log has no claim to.
    let entries = audit(&server, &admin, "pay_terms.recorded").await;
    assert_eq!(entries.len(), 3, "{entries:?}");
    let newest = &entries[0];
    assert_eq!(newest["target_id"], employee.as_str());
    assert_eq!(newest["details"]["corrects"], typo_id, "{newest}");
    assert_eq!(newest["details"]["effective_from"], "2026-03-01");
    assert_eq!(newest["details"]["basis"], "hourly");
    for entry in &entries {
        // The whole set of keys, so an amount cannot ride in under a new name.
        let mut keys: Vec<&str> = entry["details"].as_object().expect("details").keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["basis", "corrects", "currency", "effective_from", "version"],
            "the log carries no amount: {entry}"
        );
    }
    assert!(entries[1]["details"]["corrects"].is_null(), "a new start corrects nothing");

    server.close().await;
}

#[tokio::test]
async fn only_an_administrator_reads_or_writes_somebody_else_s_terms() {
    let Some(server) = TestServer::start().await else { return };
    let (admin, employee) = admin_and_employee(&server).await;
    record(&server, &admin, &employee, hourly("2026-01-01", "20")).await;

    // A manager of the employee's own department sees the days and not the
    // money.
    server.add_agent("lead@example.test", "lead-token").await;
    server.set_password("lead@example.test", "correct horse").await;
    let lead: String = sqlx::query_scalar("SELECT id::text FROM users WHERE email = 'lead@example.test'")
        .fetch_one(&server.pool)
        .await
        .unwrap();
    let (status, _, body) = server
        .patch_with_cookie(&format!("/api/v1/users/{lead}"), Some(&admin), json!({ "role": "manager" }))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, _, body) = server
        .post_with_cookie("/api/v1/departments", Some(&admin), json!({ "name": "Engineering", "manager_id": lead }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let department = body["id"].as_str().unwrap().to_string();
    let (status, _, body) = server
        .put_with_cookie(
            &format!("/api/v1/users/{employee}/department"),
            Some(&admin),
            json!({ "department_id": department }),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (_, manager, _) = server.login("lead@example.test", "correct horse").await;
    let manager = manager.expect("the manager signs in");

    let (status, body) = server
        .get_with_cookie(&format!("/api/v1/users/{employee}/days?from=2026-01-01&to=2026-01-07"), Some(&manager))
        .await;
    assert_eq!(status, StatusCode::OK, "the manager does see the days: {body}");

    let path = format!("/api/v1/users/{employee}/pay-terms");
    let (status, _) = server.get_with_cookie(&path, Some(&manager)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a manager does not read the pay");
    let (status, _, _) = server.post_with_cookie(&path, Some(&manager), hourly("2026-02-01", "99")).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "nor write it");

    let employee_cookie = employee_cookie(&server).await;
    let (status, _) = server.get_with_cookie(&format!("/api/v1/users/{lead}/pay-terms"), Some(&employee_cookie)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "an employee reads nobody else's");
    let (status, _, _) = server.post_with_cookie(&path, Some(&employee_cookie), hourly("2026-02-01", "99")).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "and does not raise their own");

    let (status, _) = server.get_with_cookie("/api/v1/me/pay-terms", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = server.get_with_cookie(&path, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Nothing was written by the refused attempts.
    assert_eq!(server.count("pay_terms").await, 1);

    // Somebody with no terms has an empty history, not an error.
    let (status, mine) = server.get_with_cookie("/api/v1/me/pay-terms", Some(&manager)).await;
    assert_eq!(status, StatusCode::OK, "{mine}");
    assert_eq!(mine["versions"], json!([]));

    let unknown = "00000000-0000-0000-0000-000000000000";
    let (status, _) = server.get_with_cookie(&format!("/api/v1/users/{unknown}/pay-terms"), Some(&admin)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = server
        .post_with_cookie(&format!("/api/v1/users/{unknown}/pay-terms"), Some(&admin), hourly("2026-02-01", "20"))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    server.close().await;
}

#[tokio::test]
async fn reading_somebody_else_s_pay_is_recorded_and_reading_your_own_is_not() {
    let Some(server) = TestServer::start().await else { return };
    let (admin, employee) = admin_and_employee(&server).await;
    record(&server, &admin, &employee, hourly("2026-01-01", "20")).await;

    let (status, _) = server.get_with_cookie(&format!("/api/v1/users/{employee}/pay-terms"), Some(&admin)).await;
    assert_eq!(status, StatusCode::OK);
    let viewed = audit(&server, &admin, "pay_terms.viewed").await;
    assert_eq!(viewed.len(), 1, "{viewed:?}");
    assert_eq!(viewed[0]["target_id"], employee.as_str());
    assert_eq!(viewed[0]["target_label"], EMPLOYEE.0);
    assert_eq!(viewed[0]["actor_email"], ADMIN.0);

    // The administrator's own terms, and the employee's own, are theirs to
    // read without leaving a trace.
    let admin_id: String = sqlx::query_scalar("SELECT id::text FROM users WHERE email = $1")
        .bind(ADMIN.0)
        .fetch_one(&server.pool)
        .await
        .unwrap();
    let (status, _) = server.get_with_cookie(&format!("/api/v1/users/{admin_id}/pay-terms"), Some(&admin)).await;
    assert_eq!(status, StatusCode::OK);
    let cookie = employee_cookie(&server).await;
    let (status, _) = server.get_with_cookie("/api/v1/me/pay-terms", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(audit(&server, &admin, "pay_terms.viewed").await.len(), 1);

    server.close().await;
}

#[tokio::test]
async fn a_version_the_column_would_change_is_refused() {
    let Some(server) = TestServer::start().await else { return };
    let (admin, employee) = admin_and_employee(&server).await;
    let path = format!("/api/v1/users/{employee}/pay-terms");

    let refused = [
        (
            json!({ "effective_from": "2026-01-01", "basis": "hourly", "rate": "0", "currency": "USD" }),
            "zero",
        ),
        (
            json!({ "effective_from": "2026-01-01", "basis": "hourly", "rate": "-3", "currency": "USD" }),
            "negative",
        ),
        (
            json!({ "effective_from": "2026-01-01", "basis": "hourly", "rate": "10.12345", "currency": "USD" }),
            "a fifth place",
        ),
        (
            json!({ "effective_from": "2026-01-01", "basis": "hourly", "rate": "ten", "currency": "USD" }),
            "not a number",
        ),
        (
            json!({ "effective_from": "2026-01-01", "basis": "hourly", "rate": "10", "currency": "USDT" }),
            "not ISO 4217",
        ),
        (
            json!({ "effective_from": "2026-01-01", "basis": "hourly", "rate": "10", "currency": "HRK" }),
            "a retired code",
        ),
        (
            json!({ "effective_from": "2026-01-01", "basis": "hourly", "rate": "10", "currency": "USD", "note": "x".repeat(501) }),
            "a long note",
        ),
    ];
    for (version, why) in refused {
        let (status, _, body) = server.post_with_cookie(&path, Some(&admin), version).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{why}: {body}");
        assert!(body["error"].is_string(), "{why} is explained: {body}");
    }

    // Shapes the decoder refuses before the handler sees them.
    let malformed = [
        (
            json!({ "effective_from": "2026-01-01", "basis": "hourly", "rate": 25.5, "currency": "USD" }),
            "a JSON number",
        ),
        (
            json!({ "effective_from": "2026-01-01", "basis": "piece", "rate": "10", "currency": "USD" }),
            "an unknown basis",
        ),
        (
            json!({ "effective_from": "2026-02-30", "basis": "hourly", "rate": "10", "currency": "USD" }),
            "a date that is not one",
        ),
        (
            json!({ "effective_from": "2026-01-01", "basis": "hourly", "rate": "10", "currency": "USD", "notes": "typo" }),
            "an unknown field",
        ),
    ];
    for (version, why) in malformed {
        let (status, _, body) = server.post_with_cookie(&path, Some(&admin), version).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{why}: {body}");
    }

    assert_eq!(server.count("pay_terms").await, 0, "nothing refused was written");

    // What is accepted is stored as sent, and written back with the currency's
    // own minor unit.
    let history = record(
        &server,
        &admin,
        &employee,
        json!({ "effective_from": "2026-01-01", "basis": "monthly", "rate": "350000", "currency": "jpy", "note": "  " }),
    )
    .await;
    let version = &history["versions"][0];
    assert_eq!(version["currency"], "JPY");
    assert_eq!(version["rate"], "350000");
    assert_eq!(version["basis"], "monthly");
    assert!(version["note"].is_null(), "a blank note is no note: {version}");

    let history = record(
        &server,
        &admin,
        &employee,
        json!({ "effective_from": "2026-02-01", "basis": "daily", "rate": "0.125", "currency": "BHD" }),
    )
    .await;
    assert_eq!(history["versions"][0]["rate"], "0.125");

    server.close().await;
}

#[tokio::test]
async fn the_database_keeps_the_history_whatever_a_handler_does() {
    let Some(server) = TestServer::start().await else { return };
    let (admin, employee) = admin_and_employee(&server).await;
    record(&server, &admin, &employee, hourly("2026-01-01", "20")).await;

    // The rule is in the schema, not in the routes: a statement written later,
    // anywhere, that rewrites or removes a version is refused.
    let update = sqlx::query("UPDATE pay_terms SET rate = 21").execute(&server.pool).await;
    let error = update.expect_err("a version is never rewritten").to_string();
    assert!(error.contains("pay terms are history"), "{error}");
    let delete = sqlx::query("DELETE FROM pay_terms").execute(&server.pool).await;
    assert!(delete.is_err(), "nor removed on its own");
    assert_eq!(server.count("pay_terms").await, 1);

    // Nor does the person who recorded it disappear from under it.
    let admin_delete = sqlx::query("DELETE FROM users WHERE email = $1").bind(ADMIN.0).execute(&server.pool).await;
    assert!(admin_delete.is_err(), "the history would lose its author");

    // The one way a version goes is with everybody, at once - the demo's
    // reseed - and then it goes with its account. Two administrators who
    // recorded each other's terms close the loop that row-by-row checking
    // cannot get out of, whichever account the delete reaches first: each is
    // still named as the author of a version whose own account is not gone
    // yet. Only a check at commit lets the reseed through.
    server.add_admin("second@example.test", "second-password").await;
    let (_, second, _) = server.login("second@example.test", "second-password").await;
    let second = second.expect("the second administrator signs in");
    let first_id: String = sqlx::query_scalar("SELECT id::text FROM users WHERE email = $1")
        .bind(ADMIN.0)
        .fetch_one(&server.pool)
        .await
        .unwrap();
    let second_id: String = sqlx::query_scalar("SELECT id::text FROM users WHERE email = 'second@example.test'")
        .fetch_one(&server.pool)
        .await
        .unwrap();
    record(&server, &second, &first_id, hourly("2026-01-01", "40")).await;
    record(&server, &admin, &second_id, hourly("2026-01-01", "40")).await;

    let mut tx = server.pool.begin().await.unwrap();
    sqlx::query("DELETE FROM users").execute(&mut *tx).await.expect("the reseed's delete");
    tx.commit().await.expect("and its commit");
    assert_eq!(server.count("pay_terms").await, 0);

    server.close().await;
}

#[tokio::test]
async fn a_restore_brings_every_version_back() {
    let Some(server) = TestServer::start().await else { return };
    let (admin, employee) = admin_and_employee(&server).await;
    record(&server, &admin, &employee, hourly("2026-01-01", "20")).await;
    record(&server, &admin, &employee, hourly("2026-03-01", "2500")).await;
    let before = record(&server, &admin, &employee, hourly("2026-03-01", "25")).await;

    let schema = kasl_server::migrator().migrations.last().map(|m| m.version).unwrap_or_default();
    let mut file = Vec::new();
    kasl_server::backup::dump(&server.pool, schema, &mut file).await.expect("the backup");
    let Some(fresh) = support::TestDb::create().await else { return };
    kasl_server::backup::load(&fresh.pool, schema, file.as_slice()).await.expect("the restore");

    let second = TestServer::wrap(fresh);
    let (_, cookie, _) = second.login(ADMIN.0, ADMIN.1).await;
    let cookie = cookie.expect("the administrator survives the restore");
    let (status, after) = second.get_with_cookie(&format!("/api/v1/users/{employee}/pay-terms"), Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{after}");
    assert_eq!(after["versions"], before["versions"], "the same versions, ids, states and recorders");

    // And the counter carries on past them: the next version is recorded
    // after the restored ones, so it takes its date over as it should.
    let (status, _, history) = second
        .post_with_cookie(&format!("/api/v1/users/{employee}/pay-terms"), Some(&cookie), hourly("2026-03-01", "26"))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{history}");
    assert_eq!(rows(&history)[0], row("2026-03-01", "26.00", "current"), "{history}");

    second.close().await;
}
