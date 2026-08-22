//! Exercises the audit log: that things are recorded, that the record is
//! readable by an administrator and nobody else, and that nothing secret
//! reaches it.
//!
//! The last one matters most. A log nobody can read is useless; a log that
//! carries a token or a password is worse than none, because it is copied into
//! tickets and screenshots by people who trust it.
//!
//! Skipped unless `DATABASE_URL` is set; CI runs them with a Postgres service.

mod support;

use axum::http::StatusCode;
use serde_json::{Value, json};
use support::TestServer;

const PASSWORD: &str = "correct horse battery staple";

async fn as_admin(server: &TestServer) -> String {
    server.add_admin("boss@example.test", PASSWORD).await;
    let (_, cookie, _) = server.login("boss@example.test", PASSWORD).await;
    cookie.expect("the admin should sign in")
}

/// The whole log, newest first.
async fn log(server: &TestServer, admin: &str) -> Value {
    let (status, body) = server.get_with_cookie("/api/v1/audit", Some(admin)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// The actions in a log response, in order.
fn actions(log: &Value) -> Vec<String> {
    log.as_array()
        .expect("a list of entries")
        .iter()
        .map(|entry| entry["action"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[tokio::test]
async fn managing_people_leaves_a_trail() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;

    let (_, _, created) = server
        .post_with_cookie("/api/v1/users", Some(&admin), json!({"email": "ivan@example.test", "role": "manager"}))
        .await;
    let ivan = created["id"].as_str().unwrap().to_string();
    server
        .patch_with_cookie(&format!("/api/v1/users/{ivan}"), Some(&admin), json!({"display_name": "Ivan"}))
        .await;

    let entries = log(&server, &admin).await;
    let actions = actions(&entries);
    assert!(actions.contains(&"user.created".to_string()), "{actions:?}");
    assert!(actions.contains(&"user.updated".to_string()), "{actions:?}");
    assert!(actions.contains(&"auth.login".to_string()), "signing in is worth recording: {actions:?}");

    // An entry has to say who, to whom, and what - a row that only says
    // "something was updated" is a row nobody can act on.
    let created = entries.as_array().unwrap().iter().find(|e| e["action"] == "user.created").unwrap();
    assert_eq!(created["actor_email"], "boss@example.test");
    assert_eq!(created["target_id"], ivan);
    assert_eq!(created["target_label"], "ivan@example.test");
    assert_eq!(created["details"]["role"], "manager");
}

#[tokio::test]
async fn the_log_never_carries_a_token_or_a_password() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    let target: String = server.scalar("SELECT id::text FROM users WHERE email = 'employee@example.test'").await;

    // Everything in one run that has a secret to leak.
    server
        .post_with_cookie("/api/v1/users", Some(&admin), json!({"email": "ivan@example.test", "password": PASSWORD}))
        .await;
    let (_, _, issued) = server
        .post_with_cookie(&format!("/api/v1/users/{target}/agents"), Some(&admin), json!({"name": "laptop"}))
        .await;
    let token = issued["token"].as_str().unwrap().to_string();
    server
        .patch_with_cookie(&format!("/api/v1/users/{target}"), Some(&admin), json!({"password": "a reset password"}))
        .await;
    server.login("ivan@example.test", "the wrong one").await;

    // The whole table as text: nothing in it may contain a credential in any
    // field, however it was nested.
    let text = log(&server, &admin).await.to_string();
    assert!(!text.contains(&token), "an issued token must never be recorded");
    assert!(!text.contains(PASSWORD), "nor a password that was set");
    assert!(!text.contains("a reset password"), "nor one an admin reset to");
    assert!(!text.contains("the wrong one"), "nor one that was typed wrongly");

    // What it does carry: that the reset happened.
    let entries = log(&server, &admin).await;
    let updated = entries.as_array().unwrap().iter().find(|e| e["action"] == "user.updated").unwrap();
    assert_eq!(updated["details"]["password_reset"], true, "{updated}");
}

#[tokio::test]
async fn a_failed_sign_in_is_recorded_with_the_address_that_was_tried() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;

    server.login("boss@example.test", "not the password").await;
    server.login("nobody@example.test", "guessing").await;

    let entries = log(&server, &admin).await;
    let failures: Vec<&Value> = entries.as_array().unwrap().iter().filter(|e| e["action"] == "auth.login_failed").collect();
    assert_eq!(failures.len(), 2, "both a wrong password and an unknown address: {failures:?}");

    // A run of failures against one address is the thing worth seeing, so the
    // attempted address is kept even when it belongs to nobody.
    let attempted: Vec<&str> = failures.iter().map(|e| e["actor_email"].as_str().unwrap_or_default()).collect();
    assert!(attempted.contains(&"boss@example.test"));
    assert!(attempted.contains(&"nobody@example.test"));
    assert!(failures.iter().all(|e| e["actor_id"].is_null()), "a failed attempt authenticates nobody");
}

#[tokio::test]
async fn issuing_and_revoking_a_token_are_both_recorded() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    let target: String = server.scalar("SELECT id::text FROM users WHERE email = 'employee@example.test'").await;

    let (_, _, issued) = server
        .post_with_cookie(&format!("/api/v1/users/{target}/agents"), Some(&admin), json!({"name": "laptop"}))
        .await;
    let agent_id = issued["id"].as_str().unwrap().to_string();
    server.delete_with_cookie(&format!("/api/v1/agents/{agent_id}"), Some(&admin)).await;

    let entries = log(&server, &admin).await;
    let issued_entry = entries.as_array().unwrap().iter().find(|e| e["action"] == "agent.issued").unwrap();
    assert_eq!(issued_entry["target_id"], agent_id);
    assert_eq!(issued_entry["target_label"], "laptop");
    assert_eq!(issued_entry["details"]["user_id"], target, "whose history this token can write: {issued_entry}");

    let revoked = entries.as_array().unwrap().iter().find(|e| e["action"] == "agent.revoked").unwrap();
    assert_eq!(revoked["target_id"], agent_id);
}

#[tokio::test]
async fn moving_someone_between_departments_is_recorded() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    let target: String = server.scalar("SELECT id::text FROM users WHERE email = 'employee@example.test'").await;

    let (_, _, created) = server
        .post_with_cookie("/api/v1/departments", Some(&admin), json!({"name": "Engineering"}))
        .await;
    let department = created["id"].as_str().unwrap().to_string();
    server
        .put_with_cookie(
            &format!("/api/v1/users/{target}/department"),
            Some(&admin),
            json!({"department_id": department}),
        )
        .await;

    // Who can see whom changes here, which is what an audit reader is usually
    // trying to reconstruct.
    let entries = log(&server, &admin).await;
    let assigned = entries.as_array().unwrap().iter().find(|e| e["action"] == "department.assigned").unwrap();
    assert_eq!(assigned["target_id"], target);
    assert_eq!(assigned["details"]["department_id"], department);

    let created_entry = entries.as_array().unwrap().iter().find(|e| e["action"] == "department.created").unwrap();
    assert_eq!(created_entry["target_label"], "Engineering");
}

#[tokio::test]
async fn only_an_administrator_may_read_the_log() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    for (email, role) in [("manager@example.test", "manager"), ("worker@example.test", "employee")] {
        server
            .post_with_cookie("/api/v1/users", Some(&admin), json!({"email": email, "role": role, "password": PASSWORD}))
            .await;
        let (_, cookie, _) = server.login(email, PASSWORD).await;
        let (status, body) = server.get_with_cookie("/api/v1/audit", cookie.as_deref()).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{role} must not read the log: {body}");
    }

    let (status, _) = server.get_with_cookie("/api/v1/audit", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // An agent's token is not a way in either.
    let (status, _) = server.get_with_header("/api/v1/audit", Some(&format!("Bearer {}", server.token))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn there_is_no_way_to_delete_from_the_log() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    server
        .post_with_cookie("/api/v1/users", Some(&admin), json!({"email": "ivan@example.test"}))
        .await;
    let before = log(&server, &admin).await.as_array().unwrap().len();

    // A journal the watched party can erase is not a journal. Even an admin -
    // the main subject of the log - gets no route for it.
    let (status, _, _) = server.delete_with_cookie("/api/v1/audit", Some(&admin)).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "no delete route may exist");

    // Nothing was removed - and nothing was added either. Reading the log is
    // not itself an auditable event: recording every read would bury the
    // actions in a log of people looking at the log.
    assert_eq!(log(&server, &admin).await.as_array().unwrap().len(), before, "the log is unchanged");
}

#[tokio::test]
async fn the_log_can_be_narrowed_to_one_person_or_one_action() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;

    let (_, _, created) = server
        .post_with_cookie("/api/v1/users", Some(&admin), json!({"email": "ivan@example.test"}))
        .await;
    let ivan = created["id"].as_str().unwrap().to_string();
    server
        .post_with_cookie("/api/v1/users", Some(&admin), json!({"email": "other@example.test"}))
        .await;

    // "Everything about this person" is the question an audit log exists to
    // answer.
    let (status, filtered) = server.get_with_cookie(&format!("/api/v1/audit?target_id={ivan}"), Some(&admin)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(filtered.as_array().unwrap().len(), 1, "{filtered}");
    assert_eq!(filtered[0]["target_label"], "ivan@example.test");

    let (_, by_action) = server.get_with_cookie("/api/v1/audit?action=user.created", Some(&admin)).await;
    assert_eq!(by_action.as_array().unwrap().len(), 2);
    assert!(by_action.as_array().unwrap().iter().all(|e| e["action"] == "user.created"));

    let (_, none) = server.get_with_cookie("/api/v1/audit?action=nothing.happened", Some(&admin)).await;
    assert!(none.as_array().unwrap().is_empty(), "an unknown action is empty, not an error");
}

#[tokio::test]
async fn a_read_of_the_log_is_bounded() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    for i in 0..5 {
        server
            .post_with_cookie("/api/v1/users", Some(&admin), json!({"email": format!("person{i}@example.test")}))
            .await;
    }

    let (_, page) = server.get_with_cookie("/api/v1/audit?limit=2", Some(&admin)).await;
    assert_eq!(page.as_array().unwrap().len(), 2);

    // Newest first, and paging back through history must not repeat or skip.
    let (_, second) = server.get_with_cookie("/api/v1/audit?limit=2&offset=2", Some(&admin)).await;
    assert_eq!(second.as_array().unwrap().len(), 2);
    assert_ne!(page[0]["id"], second[0]["id"]);
    assert!(page[0]["id"].as_i64() > second[0]["id"].as_i64(), "newest first");

    // A limit past the ceiling is clamped rather than honoured: this table
    // grows without limit and an unbounded read is a way to take the server
    // down with one request.
    //
    // Asserting on the row count would prove nothing here - a handful of
    // entries fit under any ceiling - so the clamp is observed where it is
    // visible: with more rows than the ceiling, an oversized request returns
    // exactly the ceiling.
    server
        .execute(
            "INSERT INTO audit_log (action, actor_email)
             SELECT 'test.filler', 'filler@example.test' FROM generate_series(1, 600)",
        )
        .await;

    let (status, huge) = server.get_with_cookie("/api/v1/audit?limit=100000", Some(&admin)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(huge.as_array().unwrap().len(), 500, "the ceiling must be applied, not the requested limit");
}

#[tokio::test]
async fn an_entry_survives_the_person_who_made_it() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    server
        .post_with_cookie("/api/v1/users", Some(&admin), json!({"email": "ivan@example.test"}))
        .await;

    // Accounts are deactivated rather than deleted, but if a row ever does go,
    // the record of what that person did must not go with it - that is the
    // whole point of an audit log.
    server.execute("DELETE FROM users WHERE email = 'boss@example.test'").await;

    let entries: i64 = server.scalar("SELECT count(*) FROM audit_log WHERE action = 'user.created'").await;
    assert_eq!(entries, 1, "the entry must survive");

    let actor: Option<String> = server
        .optional_scalar("SELECT actor_id::text FROM audit_log WHERE action = 'user.created'")
        .await;
    assert!(actor.is_none(), "its actor reference clears");
    let email: String = server.scalar("SELECT actor_email FROM audit_log WHERE action = 'user.created'").await;
    assert_eq!(email, "boss@example.test", "but the entry still says who it was");
}
