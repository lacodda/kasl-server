//! Reports and their approval, against a live database.
//!
//! The rules that need no database are unit-tested in the module. What is only
//! reachable here: that a report stands for the figures the day came to when
//! it was made and stops standing for the day once kasl sends other ones; who
//! may answer whose report - the drill-down's boundary; and what an answer
//! leaves behind - the notice to the person, the audit entry, the queue
//! (ADR 0022).

mod support;

use axum::http::StatusCode;
use serde_json::{Value, json};
use support::{TestDb, TestServer};

/// The cast: an administrator, a manager with a department and an agent of
/// their own, an employee in the department with an agent, and an employee
/// outside it with one.
struct Team {
    admin: Option<String>,
    manager: Option<String>,
    employee: Option<String>,
    outsider: Option<String>,
}

const INSIDE: &str = "inside-token";
const OUTSIDE: &str = "outside-token";
const LEAD: &str = "lead-token";

async fn team(server: &TestServer) -> Team {
    server.add_admin("boss@example.test", "correct horse").await;
    let (_, admin, _) = server.login("boss@example.test", "correct horse").await;

    server.add_agent("inside@example.test", INSIDE).await;
    server.add_agent("outside@example.test", OUTSIDE).await;
    server.add_agent("lead@example.test", LEAD).await;
    for email in ["inside@example.test", "outside@example.test", "lead@example.test"] {
        server.set_password(email, "correct horse").await;
    }

    let (status, body) = server.get_with_cookie("/api/v1/users", admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = |email: &str| -> String {
        body.as_array()
            .unwrap()
            .iter()
            .find(|user| user["email"] == email)
            .and_then(|user| user["id"].as_str())
            .unwrap_or_else(|| panic!("{email} is listed"))
            .to_string()
    };
    let (manager_id, inside_id) = (id("lead@example.test"), id("inside@example.test"));

    let (status, _, body) = server
        .patch_with_cookie(&format!("/api/v1/users/{manager_id}"), admin.as_deref(), json!({ "role": "manager" }))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, _, body) = server
        .post_with_cookie(
            "/api/v1/departments",
            admin.as_deref(),
            json!({ "name": "Engineering", "manager_id": manager_id }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let department_id = body["id"].as_str().unwrap().to_string();
    for member in [&inside_id, &manager_id] {
        let (status, _, body) = server
            .put_with_cookie(
                &format!("/api/v1/users/{member}/department"),
                admin.as_deref(),
                json!({ "department_id": department_id }),
            )
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    }

    let (_, manager, _) = server.login("lead@example.test", "correct horse").await;
    let (_, employee, _) = server.login("inside@example.test", "correct horse").await;
    let (_, outsider, _) = server.login("outside@example.test", "correct horse").await;

    Team {
        admin,
        manager,
        employee,
        outsider,
    }
}

/// A finished day: nine to six with half an hour's lunch - 8.5 hours.
fn day(date: &str) -> Value {
    day_until(date, "18:00:00")
}

fn day_until(date: &str, end: &str) -> Value {
    json!({
        "date": date,
        "started_at": format!("{date}T09:00:00-03:00"),
        "ended_at": format!("{date}T{end}-03:00"),
        "pauses": [
            { "started_at": format!("{date}T12:00:00-03:00"), "ended_at": format!("{date}T12:30:00-03:00"), "duration_seconds": 1800, "manual": true, "reason": "lunch" }
        ],
        "tasks": []
    })
}

async fn upload(server: &TestServer, token: &str, day: Value) {
    let (status, body) = server.post_day(token, day).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

async fn report(server: &TestServer, cookie: Option<&str>, date: &str) -> (StatusCode, Value) {
    let (status, _, body) = server.post_with_cookie("/api/v1/me/reports", cookie, json!({ "date": date })).await;
    (status, body)
}

async fn approve(server: &TestServer, cookie: Option<&str>, ids: &[&Value]) -> (StatusCode, Value) {
    let (status, _, body) = server.post_with_cookie("/api/v1/reports/approve", cookie, json!({ "ids": ids })).await;
    (status, body)
}

async fn send_back(server: &TestServer, cookie: Option<&str>, id: &Value, reason: &str) -> (StatusCode, Value) {
    let path = format!("/api/v1/reports/{}/return", id.as_str().unwrap());
    let (status, _, body) = server.post_with_cookie(&path, cookie, json!({ "reason": reason })).await;
    (status, body)
}

async fn turn_approval(server: &TestServer, admin: Option<&str>, enabled: bool) {
    let (status, _, body) = server.put_with_cookie("/api/v1/reports/approval", admin, json!({ "enabled": enabled })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["enabled"], enabled);
}

/// The report on one of the reader's own days, as their week shows it.
async fn own_report(server: &TestServer, cookie: Option<&str>, date: &str) -> Value {
    let (status, body) = server.get_with_cookie(&format!("/api/v1/me/days?from={date}&to={date}"), cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["days"][0]["report"].clone()
}

async fn queue(server: &TestServer, cookie: Option<&str>) -> Value {
    let (status, body) = server.get_with_cookie("/api/v1/team/reports", cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

async fn reports_in_db(server: &TestServer) -> i64 {
    server.scalar("SELECT count(*) FROM reports").await
}

#[tokio::test]
async fn a_finished_day_is_reported_once_at_the_figures_it_came_to() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;
    upload(&server, INSIDE, day("2026-09-28")).await;

    let (status, first) = report(&server, team.employee.as_deref(), "2026-09-28").await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    assert_eq!(first["status"], "submitted");
    assert_eq!(first["worked_seconds"], 8 * 3600 + 1800, "nine to six, less half an hour");
    assert_eq!(first["kind"], "work");
    assert!(first["reviewer"].is_null() && first["reason"].is_null(), "{first}");

    // Sent again - a retry, a second click: the standing report answers, and
    // no copy joins anybody's queue.
    let (status, again) = report(&server, team.employee.as_deref(), "2026-09-28").await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["id"], first["id"]);
    assert_eq!(reports_in_db(&server).await, 1);

    // On the day, where the person and the drill-down read it.
    let on_day = own_report(&server, team.employee.as_deref(), "2026-09-28").await;
    assert_eq!(on_day["id"], first["id"]);
    assert_eq!(on_day["status"], "submitted");

    // Nobody approves days here yet, and the days answer says so.
    let (_, week) = server
        .get_with_cookie("/api/v1/me/days?from=2026-09-28&to=2026-09-28", team.employee.as_deref())
        .await;
    assert_eq!(week["day_approval"], false);

    server.close().await;
}

#[tokio::test]
async fn kasl_reports_a_day_the_same_way() {
    let Some(server) = TestServer::start().await else { return };
    let _team = team(&server).await;
    upload(&server, INSIDE, day("2026-09-28")).await;

    let bearer = format!("Bearer {INSIDE}");
    let (status, body) = server
        .post_with_header("/api/v1/agent/reports", Some(&bearer), json!({ "date": "2026-09-28" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["status"], "submitted");

    let (status, body) = server
        .post_with_header("/api/v1/agent/reports", Some(&bearer), json!({ "date": "2026-09-28" }))
        .await;
    assert_eq!(status, StatusCode::OK, "a retry from the agent writes nothing: {body}");

    let (status, body) = server
        .post_with_header("/api/v1/agent/reports", Some("Bearer not-a-token"), json!({ "date": "2026-09-28" }))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    assert_eq!(reports_in_db(&server).await, 1);
    server.close().await;
}

#[tokio::test]
async fn only_a_finished_day_that_exists_is_reported() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    let (status, body) = report(&server, team.employee.as_deref(), "2026-09-28").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "no day was sent: {body}");

    let open = json!({
        "date": "2026-09-29",
        "started_at": "2026-09-29T09:00:00-03:00",
        "pauses": [],
        "tasks": []
    });
    upload(&server, INSIDE, open).await;
    let (status, body) = report(&server, team.employee.as_deref(), "2026-09-29").await;
    assert_eq!(status, StatusCode::CONFLICT, "an open day has no total to put a name to: {body}");
    assert!(body["error"].as_str().unwrap().contains("still open"), "{body}");

    // Somebody else's day is not reported by being named: `/me` is the
    // reader's own, and the outsider has no day on that date.
    upload(&server, INSIDE, day("2026-09-30")).await;
    let (status, body) = report(&server, team.outsider.as_deref(), "2026-09-30").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    assert_eq!(reports_in_db(&server).await, 0);
    server.close().await;
}

#[tokio::test]
async fn approval_is_off_until_an_administrator_turns_it_on() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;
    upload(&server, INSIDE, day("2026-09-28")).await;
    let (_, submitted) = report(&server, team.employee.as_deref(), "2026-09-28").await;

    let (status, body) = server.get_with_cookie("/api/v1/reports/approval", team.employee.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["enabled"], false);

    // Nothing is asked, so nothing waits and nothing can be answered.
    let waiting = queue(&server, team.manager.as_deref()).await;
    assert_eq!(waiting["day_approval"], false);
    assert_eq!(waiting["reports"], json!([]));
    let (status, body) = approve(&server, team.manager.as_deref(), &[&submitted["id"]]).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let (status, body) = send_back(&server, team.manager.as_deref(), &submitted["id"], "Why?").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    // A manager does not decide it for the installation.
    let (status, _, body) = server
        .put_with_cookie("/api/v1/reports/approval", team.manager.as_deref(), json!({ "enabled": true }))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    turn_approval(&server, team.admin.as_deref(), true).await;

    // The report sent before it was turned on waits now: it is the same
    // report, and the question it carries is new.
    let waiting = queue(&server, team.manager.as_deref()).await;
    assert_eq!(waiting["day_approval"], true);
    assert_eq!(waiting["waiting"], 1);
    assert_eq!(waiting["reports"][0]["id"], submitted["id"]);
    assert_eq!(waiting["reports"][0]["display_name"], "inside");
    assert_eq!(waiting["reports"][0]["department"], "Engineering");

    // Recorded, once: setting it to what it is changes nothing.
    turn_approval(&server, team.admin.as_deref(), true).await;
    let changes: i64 = server.scalar("SELECT count(*) FROM audit_log WHERE action = 'reports.approval_changed'").await;
    assert_eq!(changes, 1);

    server.close().await;
}

#[tokio::test]
async fn a_manager_approves_a_week_and_the_person_is_told_once() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;
    turn_approval(&server, team.admin.as_deref(), true).await;

    upload(&server, INSIDE, day("2026-09-28")).await;
    upload(&server, INSIDE, day_until("2026-09-29", "17:00:00")).await;
    let (_, monday) = report(&server, team.employee.as_deref(), "2026-09-28").await;
    let (_, tuesday) = report(&server, team.employee.as_deref(), "2026-09-29").await;

    // Oldest first: the longest-waiting answer is the next one to give.
    let waiting = queue(&server, team.manager.as_deref()).await;
    let ids: Vec<&Value> = waiting["reports"].as_array().unwrap().iter().map(|r| &r["id"]).collect();
    assert_eq!(ids, [&monday["id"], &tuesday["id"]]);

    let notices_before = server.count("notifications").await;
    let (status, outcome) = approve(&server, team.manager.as_deref(), &[&monday["id"], &tuesday["id"]]).await;
    assert_eq!(status, StatusCode::OK, "{outcome}");
    assert_eq!(outcome["approved"].as_array().unwrap().len(), 2, "{outcome}");
    assert_eq!(outcome["refused"], json!([]));
    assert!(
        outcome["approved"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["status"] == "approved" && r["reviewer"] == "lead")
    );

    // Off the queue, approved on the day.
    assert_eq!(queue(&server, team.manager.as_deref()).await["waiting"], 0);
    assert_eq!(own_report(&server, team.employee.as_deref(), "2026-09-28").await["status"], "approved");

    // One notice for the act, naming both days and the figures they were
    // approved at.
    assert_eq!(server.count("notifications").await, notices_before + 1);
    let (_, inbox) = server.get_with_cookie("/api/v1/me/notifications", team.employee.as_deref()).await;
    let notice = &inbox["notifications"][0];
    assert_eq!(notice["kind"], "report.approved", "{inbox}");
    assert_eq!(notice["title"], "lead approved 2 of your days");
    assert_eq!(notice["body"], "Approved as you reported them: 2026-09-28 (8.5 h) and 2026-09-29 (7.5 h).");
    assert_eq!(notice["link"], Value::Null, "no public address configured in the tests");
    assert_eq!(notice["approved"]["days"].as_array().unwrap().len(), 2);

    // Approving what already is approved says nothing again.
    let (status, outcome) = approve(&server, team.manager.as_deref(), &[&monday["id"]]).await;
    assert_eq!(status, StatusCode::OK, "{outcome}");
    assert_eq!(outcome["approved"][0]["status"], "approved");
    assert_eq!(server.count("notifications").await, notices_before + 1);

    // Each approval is in the audit log, about the person, once.
    let approvals: i64 = server.scalar("SELECT count(*) FROM audit_log WHERE action = 'report.approved'").await;
    assert_eq!(approvals, 2);

    server.close().await;
}

#[tokio::test]
async fn only_whoever_may_see_the_day_answers_it_and_never_its_person() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;
    turn_approval(&server, team.admin.as_deref(), true).await;

    upload(&server, OUTSIDE, day("2026-09-28")).await;
    upload(&server, LEAD, day("2026-09-28")).await;
    upload(&server, INSIDE, day("2026-09-28")).await;
    let (_, outsiders) = report(&server, team.outsider.as_deref(), "2026-09-28").await;
    let (_, managers) = report(&server, team.manager.as_deref(), "2026-09-28").await;
    let (_, insiders) = report(&server, team.employee.as_deref(), "2026-09-28").await;

    // The manager's queue: their department's, not their own, not another's.
    let waiting = queue(&server, team.manager.as_deref()).await;
    let ids: Vec<&Value> = waiting["reports"].as_array().unwrap().iter().map(|r| &r["id"]).collect();
    assert_eq!(ids, [&insiders["id"]]);

    // Each refusal is per report and does not stop the rest of the request.
    let (status, outcome) = approve(&server, team.manager.as_deref(), &[&outsiders["id"], &managers["id"], &insiders["id"]]).await;
    assert_eq!(status, StatusCode::OK, "{outcome}");
    assert_eq!(outcome["approved"].as_array().unwrap().len(), 1, "{outcome}");
    let refusal = |id: &Value| -> String {
        outcome["refused"]
            .as_array()
            .unwrap()
            .iter()
            .find(|refusal| &refusal["id"] == id)
            .and_then(|refusal| refusal["error"].as_str())
            .unwrap_or_else(|| panic!("{id} is refused: {outcome}"))
            .to_string()
    };
    assert_eq!(refusal(&outsiders["id"]), "no such report", "another department's day does not exist to them");
    assert!(refusal(&managers["id"]).contains("own day"));

    // An employee answers nothing, their own report least of all.
    let (status, body) = approve(&server, team.employee.as_deref(), &[&insiders["id"]]).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // The manager's own day is the administrator's to answer.
    let (status, outcome) = approve(&server, team.admin.as_deref(), &[&managers["id"], &outsiders["id"]]).await;
    assert_eq!(status, StatusCode::OK, "{outcome}");
    assert_eq!(outcome["approved"].as_array().unwrap().len(), 2, "{outcome}");

    // And somebody's own report is never returned by them either.
    upload(&server, LEAD, day("2026-09-29")).await;
    let (_, mine) = report(&server, team.manager.as_deref(), "2026-09-29").await;
    let (status, body) = send_back(&server, team.manager.as_deref(), &mine["id"], "Hm.").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body) = send_back(&server, team.manager.as_deref(), &outsiders["id"], "Hm.").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    server.close().await;
}

#[tokio::test]
async fn an_approval_stops_covering_a_day_that_kasl_sends_with_other_figures() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;
    turn_approval(&server, team.admin.as_deref(), true).await;

    upload(&server, INSIDE, day("2026-09-28")).await;
    let (_, first) = report(&server, team.employee.as_deref(), "2026-09-28").await;
    approve(&server, team.manager.as_deref(), &[&first["id"]]).await;

    // The same day sent again unchanged - a retry, a backfill - is still the
    // day that was approved. The upload replaces every row of it; the figures
    // are what the approval is about.
    upload(&server, INSIDE, day("2026-09-28")).await;
    assert_eq!(own_report(&server, team.employee.as_deref(), "2026-09-28").await["status"], "approved");

    // Corrected in kasl: the upload is taken as always (ADR 0004), and the
    // approval no longer covers the day.
    upload(&server, INSIDE, day_until("2026-09-28", "19:00:00")).await;
    let changed = own_report(&server, team.employee.as_deref(), "2026-09-28").await;
    assert_eq!(changed["status"], "changed", "{changed}");
    assert_eq!(changed["worked_seconds"], 8 * 3600 + 1800, "the report keeps the figures it was approved at");

    // It waits for its person now, not for a manager.
    assert_eq!(queue(&server, team.manager.as_deref()).await["waiting"], 0);
    let (_, outcome) = approve(&server, team.manager.as_deref(), &[&first["id"]]).await;
    assert!(outcome["refused"][0]["error"].as_str().unwrap().contains("changed"), "{outcome}");

    // Reported again, it is a new report, at the new figures - and the queue
    // has it.
    let (status, second) = report(&server, team.employee.as_deref(), "2026-09-28").await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
    assert_ne!(second["id"], first["id"]);
    assert_eq!(second["status"], "submitted");
    assert_eq!(second["worked_seconds"], 9 * 3600 + 1800);
    let waiting = queue(&server, team.manager.as_deref()).await;
    assert_eq!(waiting["reports"][0]["id"], second["id"]);

    // The first is history: answering it is refused, as an older report.
    let (_, outcome) = approve(&server, team.manager.as_deref(), &[&first["id"]]).await;
    assert!(outcome["refused"][0]["error"].as_str().unwrap().contains("reported again"), "{outcome}");

    // A day kasl reopens has moved as well: it has no total any more.
    let reopened = json!({ "date": "2026-09-28", "started_at": "2026-09-28T09:00:00-03:00", "pauses": [], "tasks": [] });
    upload(&server, INSIDE, reopened).await;
    assert_eq!(own_report(&server, team.employee.as_deref(), "2026-09-28").await["status"], "changed");

    server.close().await;
}

#[tokio::test]
async fn a_returned_day_says_why_and_is_reported_again() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;
    turn_approval(&server, team.admin.as_deref(), true).await;

    upload(&server, INSIDE, day("2026-09-28")).await;
    let (_, first) = report(&server, team.employee.as_deref(), "2026-09-28").await;

    let (status, body) = send_back(&server, team.manager.as_deref(), &first["id"], "   ").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "a day is not returned without a reason: {body}");

    let (status, returned) = send_back(&server, team.manager.as_deref(), &first["id"], "  Friday is missing its lunch break.\n").await;
    assert_eq!(status, StatusCode::OK, "{returned}");
    assert_eq!(returned["status"], "returned");
    assert_eq!(returned["reason"], "Friday is missing its lunch break.");

    // The person reads it on the day, and is told.
    let on_day = own_report(&server, team.employee.as_deref(), "2026-09-28").await;
    assert_eq!(on_day["status"], "returned");
    assert_eq!(on_day["reviewer"], "lead");
    assert_eq!(on_day["reason"], "Friday is missing its lunch break.");

    let (_, queue_of_agent) = server.get_with_header("/api/v1/agent/notifications", Some(&format!("Bearer {INSIDE}"))).await;
    let told = queue_of_agent["notifications"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["kind"] == "report.returned")
        .unwrap_or_else(|| panic!("the machine is told: {queue_of_agent}"))
        .clone();
    assert_eq!(told["title"], "lead returned your day of 2026-09-28");
    assert_eq!(told["body"], "Friday is missing its lunch break.");
    assert_eq!(told["returned"]["reason"], "Friday is missing its lunch break.");

    // The words are kept once, on the report, and never in the audit log.
    let in_payload: i64 = server
        .scalar("SELECT count(*) FROM notifications WHERE payload::text LIKE '%lunch break%'")
        .await;
    assert_eq!(in_payload, 0);
    let in_audit: i64 = server.scalar("SELECT count(*) FROM audit_log WHERE details::text LIKE '%lunch break%'").await;
    assert_eq!(in_audit, 0);
    let returns: i64 = server.scalar("SELECT count(*) FROM audit_log WHERE action = 'report.returned'").await;
    assert_eq!(returns, 1);

    // Answered once; not approved while it stands returned.
    let (status, body) = send_back(&server, team.manager.as_deref(), &first["id"], "Again.").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let (_, outcome) = approve(&server, team.manager.as_deref(), &[&first["id"]]).await;
    assert!(outcome["refused"][0]["error"].as_str().unwrap().contains("returned"), "{outcome}");
    assert_eq!(queue(&server, team.manager.as_deref()).await["waiting"], 0);

    // Reported again - even unchanged, which is the person saying "it is
    // right" - it is a new report waiting for an answer.
    let (status, second) = report(&server, team.employee.as_deref(), "2026-09-28").await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
    assert_eq!(second["status"], "submitted");
    assert!(second["reason"].is_null());
    assert_eq!(queue(&server, team.manager.as_deref()).await["reports"][0]["id"], second["id"]);

    server.close().await;
}

#[tokio::test]
async fn a_report_whose_day_moved_waits_for_its_person_not_for_a_manager() {
    // Nobody answered it, and the day moved since: the manager has nothing to
    // approve - the figures in the report are not the day's any more.
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;
    turn_approval(&server, team.admin.as_deref(), true).await;

    upload(&server, INSIDE, day("2026-09-28")).await;
    report(&server, team.employee.as_deref(), "2026-09-28").await;
    assert_eq!(queue(&server, team.manager.as_deref()).await["waiting"], 1);

    upload(&server, INSIDE, day_until("2026-09-28", "19:00:00")).await;
    let waiting = queue(&server, team.manager.as_deref()).await;
    assert_eq!(waiting["waiting"], 0, "{waiting}");
    assert_eq!(waiting["reports"], json!([]));

    server.close().await;
}

#[tokio::test]
async fn a_day_is_waiting_by_its_newest_report_and_by_no_older_one() {
    // A day reported, corrected and reported again, then corrected back to
    // what the first report said - and the second report returned. The first
    // report is unanswered and matches the day again, but it is history: the
    // day's newest report was sent back, and the next word is the person's.
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;
    turn_approval(&server, team.admin.as_deref(), true).await;

    upload(&server, INSIDE, day("2026-09-28")).await;
    let (_, first) = report(&server, team.employee.as_deref(), "2026-09-28").await;
    upload(&server, INSIDE, day_until("2026-09-28", "19:00:00")).await;
    let (status, second) = report(&server, team.employee.as_deref(), "2026-09-28").await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
    upload(&server, INSIDE, day("2026-09-28")).await;
    let (status, body) = send_back(&server, team.manager.as_deref(), &second["id"], "Which end is right?").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let waiting = queue(&server, team.manager.as_deref()).await;
    assert_eq!(waiting["reports"], json!([]), "the older report does not surface: {waiting}");
    assert_eq!(own_report(&server, team.employee.as_deref(), "2026-09-28").await["id"], second["id"]);

    // Nor can it be approved past its successor.
    let (_, outcome) = approve(&server, team.manager.as_deref(), &[&first["id"]]).await;
    assert!(outcome["refused"][0]["error"].as_str().unwrap().contains("reported again"), "{outcome}");

    server.close().await;
}

#[tokio::test]
async fn an_approved_day_can_still_be_sent_back() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;
    turn_approval(&server, team.admin.as_deref(), true).await;

    upload(&server, INSIDE, day("2026-09-28")).await;
    let (_, submitted) = report(&server, team.employee.as_deref(), "2026-09-28").await;
    approve(&server, team.manager.as_deref(), &[&submitted["id"]]).await;

    // Noticed on Monday, approved on Friday: the manager can still say so.
    let (status, body) = send_back(
        &server,
        team.manager.as_deref(),
        &submitted["id"],
        "I approved this too fast - the end looks wrong.",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "returned");

    server.close().await;
}

#[tokio::test]
async fn turning_approval_off_answers_nothing_and_erases_nothing() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;
    turn_approval(&server, team.admin.as_deref(), true).await;

    upload(&server, INSIDE, day("2026-09-28")).await;
    upload(&server, INSIDE, day("2026-09-29")).await;
    let (_, approved) = report(&server, team.employee.as_deref(), "2026-09-28").await;
    let (_, waiting) = report(&server, team.employee.as_deref(), "2026-09-29").await;
    approve(&server, team.manager.as_deref(), &[&approved["id"]]).await;

    turn_approval(&server, team.admin.as_deref(), false).await;
    assert_eq!(own_report(&server, team.employee.as_deref(), "2026-09-28").await["status"], "approved");
    assert_eq!(own_report(&server, team.employee.as_deref(), "2026-09-29").await["status"], "submitted");
    assert_eq!(queue(&server, team.manager.as_deref()).await["reports"], json!([]));

    // On again, the one nobody answered waits again.
    turn_approval(&server, team.admin.as_deref(), true).await;
    assert_eq!(queue(&server, team.manager.as_deref()).await["reports"][0]["id"], waiting["id"]);

    server.close().await;
}

#[tokio::test]
async fn the_figures_a_report_stands_for_are_the_ones_the_day_shows() {
    // Two definitions of a day's hours meet here: the days answer computes
    // them in Rust, the reports compare them in SQL (`workday_figures`). They
    // are held to each other on the shapes where they could part - a pause the
    // agent did not measure, seconds that are not whole, and a policy that
    // keeps pauses only as totals.
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    let fractional = json!({
        "date": "2026-09-28",
        "started_at": "2026-09-28T09:00:00.700-03:00",
        "ended_at": "2026-09-28T17:30:00.200-03:00",
        "pauses": [
            { "started_at": "2026-09-28T12:00:00-03:00", "ended_at": "2026-09-28T12:45:00-03:00", "duration_seconds": 2700, "manual": false },
            { "started_at": "2026-09-28T15:00:00-03:00", "ended_at": "2026-09-28T15:20:00-03:00", "manual": false }
        ],
        "tasks": []
    });
    upload(&server, INSIDE, fractional).await;

    let (status, _, body) = server
        .put_with_cookie("/api/v1/privacy", team.admin.as_deref(), json!({ "level": "coarse" }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    upload(&server, INSIDE, day("2026-09-29")).await;

    for date in ["2026-09-28", "2026-09-29"] {
        let (status, week) = server
            .get_with_cookie(&format!("/api/v1/me/days?from={date}&to={date}"), team.employee.as_deref())
            .await;
        assert_eq!(status, StatusCode::OK, "{week}");
        let shown = week["days"][0]["worked_seconds"].as_i64().expect("a finished day has a total");

        let (status, reported) = report(&server, team.employee.as_deref(), date).await;
        assert_eq!(status, StatusCode::CREATED, "{reported}");
        assert_eq!(reported["worked_seconds"], shown, "{date}: the report and the day disagree");
        // And the comparison that decides "changed" sees no change in a day
        // nobody touched.
        assert_eq!(own_report(&server, team.employee.as_deref(), date).await["status"], "submitted", "{date}");
    }

    server.close().await;
}

#[tokio::test]
async fn the_placeholder_table_is_replaced_only_while_it_is_empty() {
    // The first schema shipped an empty `reports` table nothing ever wrote to.
    // The migration replacing it refuses a table somebody filled by hand
    // rather than dropping rows it cannot name.
    let Some(db) = TestDb::create_before(20261006000002).await else {
        eprintln!("skipped: DATABASE_URL is not set");
        return;
    };
    sqlx::query("INSERT INTO users (email, display_name) VALUES ('a@example.test', 'A')")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO reports (user_id, kind, period_start, submitted_at) SELECT id, 'daily', date '2026-08-14', now() FROM users")
        .execute(&db.pool)
        .await
        .unwrap();

    let refused = kasl_server::migrator().run(&db.pool).await;
    assert!(refused.is_err(), "a filled placeholder is not dropped");
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM reports").fetch_one(&db.pool).await.unwrap();
    assert_eq!(kept, 1);

    // Emptied, the upgrade goes through - and an installation's notices
    // survive the new constraint on them.
    sqlx::query("DELETE FROM reports").execute(&db.pool).await.unwrap();
    sqlx::query(
        "INSERT INTO notifications (user_id, kind, payload)
         SELECT id, 'privacy.changed', '{\"privacy\":{\"from\":\"full\",\"to\":\"coarse\"}}' FROM users",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    db.upgrade().await;
    let notices: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications").fetch_one(&db.pool).await.unwrap();
    assert_eq!(notices, 1);

    db.drop().await;
}
