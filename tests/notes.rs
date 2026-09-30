//! Notes on a day, against a live database.
//!
//! The rules are unit-tested in the module. What is only reachable here is who
//! may write on whose day - the same boundary the drill-down draws, so the
//! cases are drawn from who is asking - and what a note leaves behind: the
//! notice to the person, on their machine and in their inbox, and what is
//! left of both once the note is withdrawn (ADR 0021).

mod support;

use axum::http::StatusCode;
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use support::{TestDb, TestServer};

/// The cast: an administrator, a manager with a department, an employee in it
/// with an agent, and an employee outside it.
struct Team {
    admin: Option<String>,
    manager: Option<String>,
    employee: Option<String>,
    manager_id: String,
    inside_id: String,
    outside_id: String,
}

const INSIDE_TOKEN: &str = "inside-token";

async fn team(server: &TestServer) -> Team {
    server.add_admin("boss@example.test", "correct horse").await;
    let (_, admin, _) = server.login("boss@example.test", "correct horse").await;

    server.add_agent("inside@example.test", INSIDE_TOKEN).await;
    server.add_agent("outside@example.test", "outside-token").await;
    server.add_agent("lead@example.test", "lead-token").await;
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
    let (manager_id, inside_id, outside_id) = (id("lead@example.test"), id("inside@example.test"), id("outside@example.test"));

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

    // The employee has read what setting them up told them - their machine
    // being issued - so every count below is what the notes add.
    let (_, inbox) = server.get_with_cookie("/api/v1/me/notifications", employee.as_deref()).await;
    let newest = inbox["notifications"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n["id"].as_i64())
        .max()
        .unwrap_or(0);
    let (status, _, body) = server
        .post_with_cookie("/api/v1/me/notifications/read", employee.as_deref(), json!({ "through": newest }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    Team {
        admin,
        manager,
        employee,
        manager_id,
        inside_id,
        outside_id,
    }
}

async fn write(server: &TestServer, cookie: Option<&str>, subject: &str, date: &str, text: &str) -> (StatusCode, Value) {
    let (status, _, body) = server
        .post_with_cookie(&format!("/api/v1/users/{subject}/notes"), cookie, json!({ "date": date, "text": text }))
        .await;
    (status, body)
}

async fn withdraw(server: &TestServer, cookie: Option<&str>, note: &str) -> (StatusCode, Value) {
    let (status, _, body) = server.delete_with_cookie(&format!("/api/v1/notes/{note}"), cookie).await;
    (status, body)
}

/// The notes on the employee's own week around `date`.
async fn own_notes(server: &TestServer, cookie: Option<&str>, from: &str, to: &str) -> Vec<Value> {
    let (status, body) = server.get_with_cookie(&format!("/api/v1/me/days?from={from}&to={to}"), cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["notes"].as_array().expect("the days answer carries notes").clone()
}

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

fn in_a_week() -> String {
    (Utc::now().date_naive() + Duration::days(7)).to_string()
}

#[tokio::test]
async fn a_manager_writes_on_a_day_and_the_person_reads_it() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    // A day that has not happened, with no workday: the note leave approved
    // ahead of time is.
    let date = in_a_week();
    let (status, note) = write(&server, team.manager.as_deref(), &team.inside_id, &date, "  Your day off is approved.\n").await;
    assert_eq!(status, StatusCode::CREATED, "{note}");
    assert_eq!(note["date"], date.as_str());
    assert_eq!(note["text"], "Your day off is approved.", "stored as it will be read");
    assert_eq!(note["author_id"], team.manager_id.as_str());

    // On the employee's own week, beside days there are none of.
    let notes = own_notes(&server, team.employee.as_deref(), &date, &date).await;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0]["id"], note["id"]);
    assert_eq!(notes[0]["author"], "lead", "named by the account's display name");

    // And on the drill-down, which is the same answer for somebody else.
    let (status, drill) = server
        .get_with_cookie(&format!("/api/v1/users/{}/days?from={date}&to={date}", team.inside_id), team.manager.as_deref())
        .await;
    assert_eq!(status, StatusCode::OK, "{drill}");
    assert_eq!(drill["notes"][0]["id"], note["id"]);

    // Outside the range, nothing: the notes are the range's, like the days.
    let yesterday = (Utc::now().date_naive() - Duration::days(1)).to_string();
    assert!(own_notes(&server, team.employee.as_deref(), &yesterday, &yesterday).await.is_empty());

    server.close().await;
}

#[tokio::test]
async fn the_person_is_told_on_their_machine_and_in_their_inbox() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    let date = in_a_week();
    let (status, note) = write(&server, team.manager.as_deref(), &team.inside_id, &date, "Approved.").await;
    assert_eq!(status, StatusCode::CREATED, "{note}");

    let (status, queue) = server.get_with_header("/api/v1/agent/notifications", Some(&bearer(INSIDE_TOKEN))).await;
    assert_eq!(status, StatusCode::OK, "{queue}");
    let told: Vec<&Value> = queue["notifications"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["kind"] == "note.added")
        .collect();
    assert_eq!(told.len(), 1, "{queue}");
    assert_eq!(told[0]["title"], format!("lead left a note on your day of {date}"));
    assert_eq!(told[0]["body"], "Approved.", "the words are the toast");
    assert_eq!(told[0]["note"]["id"], note["id"]);
    assert_eq!(told[0]["note"]["date"], date.as_str());
    assert_eq!(told[0]["note"]["text"], "Approved.");

    let (_, inbox) = server.get_with_cookie("/api/v1/me/notifications", team.employee.as_deref()).await;
    assert_eq!(inbox["notifications"][0]["kind"], "note.added", "{inbox}");
    assert_eq!(inbox["unread"], 1, "{inbox}");

    // The words are kept once, on the note. The notice's payload names the
    // note and not what it says, so a withdrawal has one place to empty.
    let payload: Value = server.scalar("SELECT payload FROM notifications WHERE kind = 'note.added'").await;
    assert!(payload["note"].get("text").is_none(), "{payload}");

    // Nobody else is told: the author, the administrator, the other employee.
    assert_eq!(server.count("notifications").await - notices_about_agents(&server).await, 1);

    server.close().await;
}

/// The notices the fixture's own agent issues wrote, which every test starts
/// with and none of which is about a note.
async fn notices_about_agents(server: &TestServer) -> i64 {
    server
        .scalar("SELECT count(*) FROM notifications WHERE kind IN ('agent.issued', 'agent.revoked')")
        .await
}

#[tokio::test]
async fn only_whoever_may_see_the_day_may_write_on_it() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;
    let date = in_a_week();

    // A manager outside the person's department is answered as if the person
    // did not exist - the drill-down's rule (ADR 0009).
    let (status, body) = write(&server, team.manager.as_deref(), &team.outside_id, &date, "Approved.").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    // An employee writes on nobody's day, their colleague's or their own.
    let (status, body) = write(&server, team.employee.as_deref(), &team.inside_id, &date, "Approved.").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // A manager sees their own days, and still cannot write on them: the
    // notice would go to its own author.
    let (status, body) = write(&server, team.manager.as_deref(), &team.manager_id, &date, "Note to self.").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // The administrator sees everybody.
    let (status, body) = write(&server, team.admin.as_deref(), &team.outside_id, &date, "Approved.").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // Nobody signed in, nothing.
    let (status, body) = write(&server, None, &team.inside_id, &date, "Approved.").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    let notes: i64 = server.scalar("SELECT count(*) FROM day_notes").await;
    assert_eq!(notes, 1, "only the administrator's note was written");

    server.close().await;
}

#[tokio::test]
async fn a_note_that_is_not_one_is_refused_with_the_reason() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;
    let cookie = team.manager.as_deref();

    let (status, body) = write(&server, cookie, &team.inside_id, &in_a_week(), "   ").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, body) = write(&server, cookie, &team.inside_id, &in_a_week(), &"x".repeat(1001)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("1000"), "{body}");

    let two_years = (Utc::now().date_naive() + Duration::days(730)).to_string();
    let (status, body) = write(&server, cookie, &team.inside_id, &two_years, "Approved.").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // A deactivated account has nobody left to read it.
    let (status, _, body) = server
        .patch_with_cookie(&format!("/api/v1/users/{}", team.inside_id), team.admin.as_deref(), json!({ "active": false }))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, body) = write(&server, cookie, &team.inside_id, &in_a_week(), "Approved.").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    let notes: i64 = server.scalar("SELECT count(*) FROM day_notes").await;
    assert_eq!(notes, 0);

    server.close().await;
}

#[tokio::test]
async fn a_withdrawn_note_loses_its_words_everywhere() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;
    let date = in_a_week();

    let (_, note) = write(&server, team.manager.as_deref(), &team.inside_id, &date, "Approved - by mistake, wrong person.").await;
    let id = note["id"].as_str().unwrap();

    let (status, body) = withdraw(&server, team.manager.as_deref(), id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["withdrawn_at"].is_string(), "{body}");

    // Off the day.
    assert!(own_notes(&server, team.employee.as_deref(), &date, &date).await.is_empty());

    // Not toasted on a machine that had not shown it yet.
    let (_, queue) = server.get_with_header("/api/v1/agent/notifications", Some(&bearer(INSIDE_TOKEN))).await;
    assert!(!queue["notifications"].as_array().unwrap().iter().any(|n| n["kind"] == "note.added"), "{queue}");
    let (_, pulse) = server
        .post_with_header(
            "/api/v1/agent/heartbeat",
            Some(&bearer(INSIDE_TOKEN)),
            json!({ "state": "working", "at": Utc::now().to_rfc3339() }),
        )
        .await;
    assert_eq!(
        pulse["notifications"],
        queue["notifications"].as_array().unwrap().len(),
        "the count and the list agree"
    );

    // Kept in the inbox, as withdrawn, without the words - and not asking for
    // attention.
    let (_, inbox) = server.get_with_cookie("/api/v1/me/notifications", team.employee.as_deref()).await;
    let notice = inbox["notifications"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["kind"] == "note.added")
        .expect("the notice is kept");
    assert!(notice["withdrawn_at"].is_string(), "{notice}");
    assert_eq!(notice["body"], "The note was withdrawn.");
    assert!(notice["note"].get("text").is_none(), "{notice}");
    assert_eq!(inbox["unread"], 0, "{inbox}");

    // And nowhere in the database: not the note, not the notice, not the
    // audit log a withdrawal cannot touch.
    let anywhere: i64 = server
        .scalar(
            "SELECT (SELECT count(*) FROM day_notes WHERE text LIKE '%wrong person%')
                  + (SELECT count(*) FROM notifications WHERE payload::text LIKE '%wrong person%')
                  + (SELECT count(*) FROM audit_log WHERE details::text LIKE '%wrong person%')",
        )
        .await;
    assert_eq!(anywhere, 0);

    // Twice is said plainly.
    let (status, body) = withdraw(&server, team.manager.as_deref(), id).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    server.close().await;
}

#[tokio::test]
async fn only_the_author_or_an_administrator_withdraws_a_note() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;
    let date = in_a_week();

    let (_, by_admin) = write(&server, team.admin.as_deref(), &team.inside_id, &date, "Approved.").await;
    let (_, by_manager) = write(&server, team.manager.as_deref(), &team.inside_id, &date, "Thanks.").await;

    // The manager sees the administrator's note on the day, and is told why
    // they cannot take it back.
    let (status, body) = withdraw(&server, team.manager.as_deref(), by_admin["id"].as_str().unwrap()).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // The person whose day it is cannot take back what was said to them.
    let (status, body) = withdraw(&server, team.employee.as_deref(), by_manager["id"].as_str().unwrap()).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // The administrator can take back anybody's.
    let (status, body) = withdraw(&server, team.admin.as_deref(), by_manager["id"].as_str().unwrap()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // A note that does not exist is a 404, the same as one the reader cannot
    // see.
    let (status, body) = withdraw(&server, team.admin.as_deref(), "00000000-0000-0000-0000-000000000000").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    server.close().await;
}

#[tokio::test]
async fn a_note_on_a_day_the_reader_cannot_see_does_not_exist_to_them() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    let (_, note) = write(&server, team.admin.as_deref(), &team.outside_id, &in_a_week(), "Approved.").await;
    let (status, body) = withdraw(&server, team.manager.as_deref(), note["id"].as_str().unwrap()).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a note on a day the reader cannot see does not exist to them: {body}"
    );

    server.close().await;
}

#[tokio::test]
async fn writing_and_withdrawing_are_in_the_audit_log_without_the_words() {
    let Some(server) = TestServer::start().await else { return };
    let team = team(&server).await;

    let (_, note) = write(&server, team.manager.as_deref(), &team.inside_id, &in_a_week(), "Approved.").await;
    withdraw(&server, team.manager.as_deref(), note["id"].as_str().unwrap()).await;

    let (status, log) = server.get_with_cookie("/api/v1/audit", team.admin.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "{log}");
    let entries = log.as_array().expect("a list of entries");
    for action in ["note.added", "note.withdrawn"] {
        let entry = entries
            .iter()
            .find(|entry| entry["action"] == action)
            .unwrap_or_else(|| panic!("{action} is recorded: {log}"));
        assert_eq!(entry["target_id"], team.inside_id.as_str(), "about the person whose day it is");
        assert_eq!(entry["details"]["note_id"], note["id"]);
        assert!(!entry.to_string().contains("Approved"), "never the words: {entry}");
    }

    server.close().await;
}

#[tokio::test]
async fn an_installation_on_0_24_upgrades_with_its_notices_intact() {
    // The shape of test the 0.24.1 pitfall asked for: a database on the
    // previous schema, holding the rows the new constraint is checked
    // against, upgraded in place. Every other test starts empty, where a
    // constraint added to a populated table has nothing to refuse.
    let Some(db) = TestDb::create_before(20260930000001).await else {
        eprintln!("skipped: DATABASE_URL is not set");
        return;
    };
    sqlx::query("INSERT INTO users (email, display_name) VALUES ('a@example.test', 'A')")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO notifications (user_id, kind, payload)
         SELECT id, 'privacy.changed', '{\"privacy\":{\"from\":\"full\",\"to\":\"coarse\"}}' FROM users",
    )
    .execute(&db.pool)
    .await
    .unwrap();

    db.upgrade().await;

    let notices: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE note_id IS NULL")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(notices, 1, "the notice already there survives the new constraint");

    db.drop().await;
}
