//! Exercises managing people and issuing agent tokens.
//!
//! The questions that need a database are the ones about authority and
//! consequence: that an employee cannot read the team, that a manager cannot
//! change it, that an issued token actually uploads and a revoked one stops,
//! and that the installation cannot be left with nobody able to administer it.
//!
//! Skipped unless `DATABASE_URL` is set; CI runs them with a Postgres service.

mod support;

use axum::http::StatusCode;
use serde_json::json;
use support::TestServer;

const PASSWORD: &str = "correct horse battery staple";

/// A server with an admin signed in, returning their cookie.
async fn as_admin(server: &TestServer) -> String {
    server.add_admin("boss@example.test", PASSWORD).await;
    let (_, cookie, _) = server.login("boss@example.test", PASSWORD).await;
    cookie.expect("the admin should be able to sign in")
}

/// Signs in someone the admin has just created.
async fn sign_in(server: &TestServer, email: &str) -> String {
    let (status, cookie, body) = server.login(email, PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{email} should be able to sign in: {body}");
    cookie.expect("a signed-in user has a cookie")
}

#[tokio::test]
async fn an_admin_creates_a_person_who_can_then_sign_in() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;

    let (status, _, body) = server
        .post_with_cookie(
            "/api/v1/users",
            Some(&admin),
            json!({"email": "new@example.test", "display_name": "New Person", "password": PASSWORD}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let cookie = sign_in(&server, "new@example.test").await;
    let (_, me) = server.get_with_cookie("/api/v1/auth/me", Some(&cookie)).await;
    assert_eq!(me["display_name"], "New Person");
    assert_eq!(me["role"], "employee", "a role has to be asked for, never assumed");
}

#[tokio::test]
async fn an_account_created_without_a_password_cannot_be_signed_into() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;

    // The common case: someone who only needs an agent reporting for them.
    let (status, _, _) = server
        .post_with_cookie("/api/v1/users", Some(&admin), json!({"email": "agent-only@example.test"}))
        .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _, _) = server.login("agent-only@example.test", PASSWORD).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (_, list) = server.get_with_cookie("/api/v1/users", Some(&admin)).await;
    let created = list.as_array().unwrap().iter().find(|row| row["email"] == "agent-only@example.test").unwrap();
    assert_eq!(created["has_password"], false, "the list must show that this account has no way in");
}

#[tokio::test]
async fn the_same_email_twice_is_a_conflict_not_a_crash() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;

    let body = json!({"email": "twice@example.test"});
    let (first, _, _) = server.post_with_cookie("/api/v1/users", Some(&admin), body.clone()).await;
    assert_eq!(first, StatusCode::CREATED);

    // Different case, same person: the unique index is on lower(email).
    let (second, _, message) = server
        .post_with_cookie("/api/v1/users", Some(&admin), json!({"email": "TWICE@example.test"}))
        .await;
    assert_eq!(second, StatusCode::CONFLICT, "{message}");
    assert!(
        message["error"].as_str().unwrap_or_default().contains("already exists"),
        "the admin should learn what went wrong: {message}"
    );
}

#[tokio::test]
async fn a_typo_for_an_email_is_refused() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;

    for bad in ["kirill", "kirill@", "kirill@example"] {
        let (status, _, _) = server.post_with_cookie("/api/v1/users", Some(&admin), json!({"email": bad})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "`{bad}` should not become an account");
    }
    let (status, _, message) = server
        .post_with_cookie("/api/v1/users", Some(&admin), json!({"email": "ok@example.test", "password": "short"}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{message}");
    assert!(message["error"].as_str().unwrap_or_default().contains("at least 8"));
}

#[tokio::test]
async fn a_manager_reads_the_team_and_changes_nothing() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;

    server
        .post_with_cookie(
            "/api/v1/users",
            Some(&admin),
            json!({"email": "manager@example.test", "role": "manager", "password": PASSWORD}),
        )
        .await;
    let manager = sign_in(&server, "manager@example.test").await;

    let (status, list) = server.get_with_cookie("/api/v1/users", Some(&manager)).await;
    assert_eq!(status, StatusCode::OK, "a manager has a dashboard to build: {list}");
    assert!(list.as_array().unwrap().len() >= 2);

    // Everything that changes the team belongs to an admin until departments
    // give a manager a scope to be in charge of.
    let (status, _, _) = server
        .post_with_cookie("/api/v1/users", Some(&manager), json!({"email": "sneaky@example.test"}))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a manager must not create people");

    let target: String = server.scalar("SELECT id::text FROM users WHERE email = 'manager@example.test'").await;
    let (status, _, _) = server
        .post_with_cookie(&format!("/api/v1/users/{target}/agents"), Some(&manager), json!({"name": "laptop"}))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "nor issue tokens, which write someone's history");
}

#[tokio::test]
async fn an_employee_cannot_see_the_team_at_all() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    server
        .post_with_cookie("/api/v1/users", Some(&admin), json!({"email": "worker@example.test", "password": PASSWORD}))
        .await;
    let worker = sign_in(&server, "worker@example.test").await;

    let (status, body) = server.get_with_cookie("/api/v1/users", Some(&worker)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "who else works here is not an employee's to enumerate");
    assert_eq!(body["error"], "not allowed", "and the refusal says nothing about what the route does");
}

#[tokio::test]
async fn an_issued_token_uploads_and_a_revoked_one_stops() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    let target: String = server.scalar("SELECT id::text FROM users WHERE email = 'employee@example.test'").await;

    let (status, _, issued) = server
        .post_with_cookie(&format!("/api/v1/users/{target}/agents"), Some(&admin), json!({"name": "laptop"}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{issued}");
    let token = issued["token"].as_str().expect("the token is returned once").to_string();
    assert!(token.starts_with("kasl_"), "{token}");

    // The point of the whole route: this token is a working credential.
    let day = json!({"date": "2026-08-20", "started_at": "2026-08-20T09:00:00-03:00"});
    let (status, body) = server.post_day(&token, day.clone()).await;
    assert_eq!(status, StatusCode::OK, "a freshly issued token must work: {body}");

    let agent_id = issued["id"].as_str().unwrap();
    let (status, _, _) = server.delete_with_cookie(&format!("/api/v1/agents/{agent_id}"), Some(&admin)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = server.post_day(&token, day).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "a revoked token must stop working at once");
}

#[tokio::test]
async fn the_token_is_never_shown_again() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    let target: String = server.scalar("SELECT id::text FROM users WHERE email = 'employee@example.test'").await;

    let (_, _, issued) = server
        .post_with_cookie(&format!("/api/v1/users/{target}/agents"), Some(&admin), json!({"name": "laptop"}))
        .await;
    let token = issued["token"].as_str().unwrap().to_string();

    let (status, listed) = server.get_with_cookie(&format!("/api/v1/users/{target}/agents"), Some(&admin)).await;
    assert_eq!(status, StatusCode::OK);
    let text = listed.to_string();
    assert!(!text.contains(&token), "the listing must not carry the token: {text}");
    assert!(text.contains("laptop"), "only what is safe to show: {text}");

    // And the database holds a hash, not the thing itself.
    let stored: String = server.scalar("SELECT token_hash FROM agents WHERE name = 'laptop'").await;
    assert_ne!(stored, token);
    assert_eq!(stored.len(), 64, "a SHA-256 in hex");
}

#[tokio::test]
async fn revoking_twice_does_not_move_when_access_ended() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    let target: String = server.scalar("SELECT id::text FROM users WHERE email = 'employee@example.test'").await;
    let (_, _, issued) = server
        .post_with_cookie(&format!("/api/v1/users/{target}/agents"), Some(&admin), json!({"name": "laptop"}))
        .await;
    let agent_id = issued["id"].as_str().unwrap().to_string();

    server.delete_with_cookie(&format!("/api/v1/agents/{agent_id}"), Some(&admin)).await;
    let first: String = server.scalar("SELECT revoked_at::text FROM agents WHERE name = 'laptop'").await;

    // Idempotent, but not by pretending it happened again: when access ended is
    // a fact, and a second click must not rewrite it.
    let (status, _, _) = server.delete_with_cookie(&format!("/api/v1/agents/{agent_id}"), Some(&admin)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let second: String = server.scalar("SELECT revoked_at::text FROM agents WHERE name = 'laptop'").await;
    assert_eq!(first, second, "the moment access ended must not move");
}

#[tokio::test]
async fn an_agent_cannot_be_issued_to_a_deactivated_person() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    let target: String = server.scalar("SELECT id::text FROM users WHERE email = 'employee@example.test'").await;

    server
        .patch_with_cookie(&format!("/api/v1/users/{target}"), Some(&admin), json!({"active": false}))
        .await;

    // It would be refused on its first upload anyway; saying so now saves
    // someone installing kasl on a laptop to find out.
    let (status, _, message) = server
        .post_with_cookie(&format!("/api/v1/users/{target}/agents"), Some(&admin), json!({"name": "laptop"}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{message}");
}

#[tokio::test]
async fn deactivating_someone_ends_their_sessions_and_stops_their_agents() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    server
        .post_with_cookie("/api/v1/users", Some(&admin), json!({"email": "leaver@example.test", "password": PASSWORD}))
        .await;
    let leaver = sign_in(&server, "leaver@example.test").await;
    let target: String = server.scalar("SELECT id::text FROM users WHERE email = 'leaver@example.test'").await;

    let (_, _, issued) = server
        .post_with_cookie(&format!("/api/v1/users/{target}/agents"), Some(&admin), json!({"name": "laptop"}))
        .await;
    let token = issued["token"].as_str().unwrap().to_string();

    // Someone's last day, in one PATCH.
    let (status, _, _) = server
        .patch_with_cookie(&format!("/api/v1/users/{target}"), Some(&admin), json!({"active": false}))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = server.get_with_cookie("/api/v1/auth/me", Some(&leaver)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "their browser session must stop working");

    // And it must be *gone*, not merely refused. Being turned away by the
    // `active` check in the session lookup is a second barrier; if the rows
    // survived, reactivating the account later would silently hand back a
    // session from before they left.
    assert_eq!(
        server.count("sessions").await,
        1,
        "only the admin's own session should remain; the leaver's rows must be deleted"
    );

    let day = json!({"date": "2026-08-20", "started_at": "2026-08-20T09:00:00-03:00"});
    let (status, _) = server.post_day(&token, day).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "and their agent must stop being accepted");
}

#[tokio::test]
async fn the_last_administrator_cannot_lock_everyone_out() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    let target: String = server.scalar("SELECT id::text FROM users WHERE email = 'boss@example.test'").await;

    // Both routes to an installation nobody can administer.
    for patch in [json!({"role": "employee"}), json!({"active": false})] {
        let (status, _, message) = server.patch_with_cookie(&format!("/api/v1/users/{target}"), Some(&admin), patch.clone()).await;
        assert_eq!(status, StatusCode::CONFLICT, "{patch} should be refused: {message}");
        assert!(message["error"].as_str().unwrap_or_default().contains("only administrator"), "{message}");
    }

    // With a second admin in place, the first may step down.
    server
        .post_with_cookie(
            "/api/v1/users",
            Some(&admin),
            json!({"email": "second@example.test", "role": "admin", "password": PASSWORD}),
        )
        .await;
    let (status, _, message) = server
        .patch_with_cookie(&format!("/api/v1/users/{target}"), Some(&admin), json!({"role": "employee"}))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{message}");
}

#[tokio::test]
async fn a_patch_changes_only_what_it_names() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    server
        .post_with_cookie(
            "/api/v1/users",
            Some(&admin),
            json!({"email": "person@example.test", "display_name": "Before", "role": "manager", "password": PASSWORD}),
        )
        .await;
    let target: String = server.scalar("SELECT id::text FROM users WHERE email = 'person@example.test'").await;

    let (status, _, _) = server
        .patch_with_cookie(&format!("/api/v1/users/{target}"), Some(&admin), json!({"display_name": "After"}))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // The fields the admin did not mention must survive: a rename that quietly
    // demoted someone would be found out weeks later.
    let (_, list) = server.get_with_cookie("/api/v1/users", Some(&admin)).await;
    let row = list.as_array().unwrap().iter().find(|row| row["email"] == "person@example.test").unwrap();
    assert_eq!(row["display_name"], "After");
    assert_eq!(row["role"], "manager", "the role must be untouched");
    assert_eq!(row["active"], true);
    assert_eq!(row["has_password"], true, "and they must still be able to sign in");
}

#[tokio::test]
async fn changing_a_password_needs_the_old_one_and_ends_the_other_sessions() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    server
        .post_with_cookie("/api/v1/users", Some(&admin), json!({"email": "person@example.test", "password": PASSWORD}))
        .await;

    // Two browsers, as after a suspicion that someone else has been signing in.
    let other = sign_in(&server, "person@example.test").await;
    let mine = sign_in(&server, "person@example.test").await;

    let (status, _, message) = server
        .post_with_cookie(
            "/api/v1/auth/password",
            Some(&mine),
            json!({"current": "not it", "new": "a whole new password"}),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "a borrowed unlocked laptop must not be enough: {message}");

    let (status, _, message) = server
        .post_with_cookie(
            "/api/v1/auth/password",
            Some(&mine),
            json!({"current": PASSWORD, "new": "a whole new password"}),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{message}");

    // The session that made the change survives; the other one does not.
    let (status, _) = server.get_with_cookie("/api/v1/auth/me", Some(&mine)).await;
    assert_eq!(status, StatusCode::OK, "being logged out of the browser you just used would be a poor reward");
    let (status, _) = server.get_with_cookie("/api/v1/auth/me", Some(&other)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "the other session is the one being defended against");

    let (status, _, _) = server.login("person@example.test", "a whole new password").await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = server.login("person@example.test", PASSWORD).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "the old password must be gone");
}

#[tokio::test]
async fn an_admin_resetting_a_password_ends_that_persons_sessions() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    server
        .post_with_cookie("/api/v1/users", Some(&admin), json!({"email": "person@example.test", "password": PASSWORD}))
        .await;
    let theirs = sign_in(&server, "person@example.test").await;
    let target: String = server.scalar("SELECT id::text FROM users WHERE email = 'person@example.test'").await;

    server
        .patch_with_cookie(&format!("/api/v1/users/{target}"), Some(&admin), json!({"password": "a reset password"}))
        .await;

    // A reset is usually a response to something being wrong; leaving open
    // sessions behind would defeat it.
    let (status, _) = server.get_with_cookie("/api/v1/auth/me", Some(&theirs)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = server.login("person@example.test", "a reset password").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn the_admin_routes_are_closed_to_callers_without_a_session() {
    let Some(server) = TestServer::start().await else { return };

    let (status, _) = server.get_with_cookie("/api/v1/users", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // An agent's bearer token is not a way in either: it authenticates a
    // machine reporting hours, not a person administering a company.
    let (status, _) = server.get_with_header("/api/v1/users", Some(&format!("Bearer {}", server.token))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
