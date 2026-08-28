//! Exercises signing in, staying signed in, and being cut off.
//!
//! Written from the browser's side: post credentials, carry the cookie the
//! server sets, call a protected route with it. What needs a database here is
//! everything that only exists once a session row does - that ending a session
//! takes effect at once, that a deactivated employee stops being let in, that
//! one person's cookie is no use to another.
//!
//! Skipped unless `DATABASE_URL` is set; CI runs them with a Postgres service.

mod support;

use axum::http::StatusCode;
use serde_json::json;
use support::TestServer;

/// The password used throughout. Long enough to pass the floor, and obviously
/// not a real one.
const PASSWORD: &str = "correct horse battery staple";

#[tokio::test]
async fn signing_in_returns_a_cookie_that_opens_the_door() {
    let Some(server) = TestServer::start().await else { return };
    server.add_admin("boss@example.test", PASSWORD).await;

    let (status, cookie, body) = server.login("boss@example.test", PASSWORD).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let cookie = cookie.expect("signing in must set a cookie");

    // The properties that make the token useless to anyone who is not the
    // browser it was handed to.
    assert!(cookie.contains("HttpOnly"), "{cookie}");
    assert!(cookie.contains("SameSite=Strict"), "{cookie}");

    let (status, body) = server.get_with_cookie("/api/v1/auth/me", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "the cookie should identify the caller: {body}");
    assert_eq!(body["email"], "boss@example.test");
    assert_eq!(body["role"], "admin");
}

#[tokio::test]
async fn a_wrong_password_and_an_unknown_email_are_the_same_answer() {
    let Some(server) = TestServer::start().await else { return };
    server.add_admin("boss@example.test", PASSWORD).await;

    let (wrong_status, _, wrong_body) = server.login("boss@example.test", "not the password").await;
    let (unknown_status, _, unknown_body) = server.login("nobody@example.test", PASSWORD).await;

    assert_eq!(wrong_status, StatusCode::UNAUTHORIZED);
    assert_eq!(unknown_status, StatusCode::UNAUTHORIZED);
    // Telling them apart would turn the login form into a way to find out who
    // works here.
    assert_eq!(wrong_body["error"], unknown_body["error"], "the two refusals must be indistinguishable");
}

#[tokio::test]
async fn an_account_without_a_password_cannot_be_signed_into() {
    let Some(server) = TestServer::start().await else { return };

    // Exactly what KASL_AGENTS creates: a user who owns data and has no way in
    // of their own. An empty password must not become a key to it.
    let (status, _, _) = server.login("employee@example.test", "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _, _) = server.login("employee@example.test", PASSWORD).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_protected_route_refuses_a_caller_without_a_session() {
    let Some(server) = TestServer::start().await else { return };

    for (label, cookie) in [
        ("no cookie", None),
        ("a made-up token", Some("kasl_session=deadbeef".to_string())),
        ("someone else's cookie name", Some("other_session=deadbeef".to_string())),
    ] {
        let (status, _) = server.get_with_cookie("/api/v1/auth/me", cookie.as_deref()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{label} must not be let in");
    }
}

#[tokio::test]
async fn logging_out_ends_the_session_immediately() {
    let Some(server) = TestServer::start().await else { return };
    server.add_admin("boss@example.test", PASSWORD).await;
    let (_, cookie, _) = server.login("boss@example.test", PASSWORD).await;
    let cookie = cookie.unwrap();

    let (status, cleared, _) = server.post_with_cookie("/api/v1/auth/logout", Some(&cookie), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(cleared.expect("logout should clear the cookie").contains("Max-Age=0"));

    // The point of server-side sessions: the same token is dead on the next
    // request, not merely forgotten by a cooperative browser.
    let (status, _) = server.get_with_cookie("/api/v1/auth/me", Some(&cookie)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "the token must stop working, not just be dropped");
    assert_eq!(server.count("sessions").await, 0);
}

#[tokio::test]
async fn logging_out_everywhere_ends_the_other_sessions_too() {
    let Some(server) = TestServer::start().await else { return };
    server.add_admin("boss@example.test", PASSWORD).await;

    // Two browsers: the laptop left on a train, and the one in hand.
    let (_, laptop, _) = server.login("boss@example.test", PASSWORD).await;
    let (_, desktop, _) = server.login("boss@example.test", PASSWORD).await;
    let (laptop, desktop) = (laptop.unwrap(), desktop.unwrap());
    assert_eq!(server.count("sessions").await, 2);

    let (status, _, body) = server.post_with_cookie("/api/v1/auth/logout-everywhere", Some(&desktop), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ended"], 2, "both sessions, including the one asking");

    for (label, cookie) in [("the laptop", &laptop), ("the desktop", &desktop)] {
        let (status, _) = server.get_with_cookie("/api/v1/auth/me", Some(cookie)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{label} must be signed out");
    }
    assert_eq!(server.count("sessions").await, 0);
}

#[tokio::test]
async fn deactivating_an_employee_closes_their_session() {
    let Some(server) = TestServer::start().await else { return };
    server.add_admin("boss@example.test", PASSWORD).await;
    let (_, cookie, _) = server.login("boss@example.test", PASSWORD).await;
    let cookie = cookie.unwrap();

    // What happens on someone's last day. The session row still exists; being
    // deactivated has to be enough on its own.
    server.execute("UPDATE users SET active = false WHERE email = 'boss@example.test'").await;

    let (status, _) = server.get_with_cookie("/api/v1/auth/me", Some(&cookie)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "a deactivated account must not keep working from an open tab");
}

#[tokio::test]
async fn an_expired_session_is_refused_even_before_it_is_swept() {
    let Some(server) = TestServer::start().await else { return };
    server.add_admin("boss@example.test", PASSWORD).await;
    let (_, cookie, _) = server.login("boss@example.test", PASSWORD).await;
    let cookie = cookie.unwrap();

    // Aged past its expiry without deleting it: the check must be in the query,
    // not in the cleanup job that may not have run.
    server.execute("UPDATE sessions SET expires_at = now() - interval '1 hour'").await;

    let (status, _) = server.get_with_cookie("/api/v1/auth/me", Some(&cookie)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(server.count("sessions").await, 1, "and the row is still there, refused rather than missing");

    let swept = kasl_server::session::sweep_expired(&server.pool).await.unwrap();
    assert_eq!(swept, 1, "the sweep is what actually removes it");
    assert_eq!(server.count("sessions").await, 0);
}

#[tokio::test]
async fn a_session_in_use_keeps_itself_alive() {
    let Some(server) = TestServer::start().await else { return };
    server.add_admin("boss@example.test", PASSWORD).await;
    let (_, cookie, _) = server.login("boss@example.test", PASSWORD).await;
    let cookie = cookie.unwrap();

    // Wind the clock forward on the row: a session used daily should not fall
    // off a cliff a fortnight after it was created.
    server.execute("UPDATE sessions SET expires_at = now() + interval '1 hour'").await;

    let (status, _) = server.get_with_cookie("/api/v1/auth/me", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);

    let days: f64 = server
        .scalar("SELECT (extract(epoch FROM (expires_at - now())) / 86400)::float8 FROM sessions")
        .await;
    assert!(days > 13.0, "using the session should push its expiry out again, got {days} days");
}

#[tokio::test]
async fn an_employee_is_not_an_admin() {
    let Some(server) = TestServer::start().await else { return };

    // The seeded agent user, given a password but left as an employee.
    server.set_password("employee@example.test", PASSWORD).await;
    let (status, cookie, _) = server.login("employee@example.test", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = server.get_with_cookie("/api/v1/auth/me", cookie.as_deref()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["role"], "employee", "signing in must not confer a role nobody granted");
}

#[tokio::test]
async fn an_agent_token_is_not_a_session_and_a_session_is_not_an_agent_token() {
    let Some(server) = TestServer::start().await else { return };
    server.add_admin("boss@example.test", PASSWORD).await;
    let (_, cookie, _) = server.login("boss@example.test", PASSWORD).await;

    // A browser session must not be able to upload as an agent...
    let day = json!({"date": "2026-08-18", "started_at": "2026-08-18T09:00:00-03:00"});
    let (status, _) = server.post_day_with_cookie(cookie.as_deref(), day.clone()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "the ingest routes take a bearer token, not a cookie");

    // ...and an agent's token must not open the browser routes.
    let (status, _) = server.get_with_header("/api/v1/auth/me", Some(&format!("Bearer {}", server.token))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "an agent has no session and no reason to have one");
}

#[tokio::test]
async fn the_admin_bootstrap_creates_and_then_resets() {
    let Some(server) = TestServer::start().await else { return };

    kasl_server::provision::ensure_admin(&server.pool, "boss@example.test", PASSWORD)
        .await
        .expect("the first admin should be created");
    let (status, _, _) = server.login("boss@example.test", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);

    // Run again with a different password: the way back in for an operator who
    // locked themselves out. It must not fail on the existing row, and the old
    // password must stop working.
    kasl_server::provision::ensure_admin(&server.pool, "boss@example.test", "an entirely different one")
        .await
        .expect("a second run should reset the password");

    let (status, _, _) = server.login("boss@example.test", PASSWORD).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "the old password must be gone");
    let (status, _, _) = server.login("boss@example.test", "an entirely different one").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(server.count("users").await, 2, "and no second account for the same person");
}

#[tokio::test]
async fn an_installation_with_no_administrator_gets_one() {
    let Some(server) = TestServer::start().await else { return };

    // A fresh install: an agent account exists (the seed), but nobody can
    // administer the server. Without this the operator has an installation
    // they cannot sign into at all.
    let password = kasl_server::provision::ensure_some_admin(&server.pool, "admin@kasl.local")
        .await
        .expect("the first run should create an administrator")
        .expect("and report the password it generated");

    assert!(password.len() >= 16, "a generated password has to be unguessable: {password}");
    let (status, cookie, body) = server.login("admin@kasl.local", &password).await;
    assert_eq!(status, StatusCode::OK, "the generated password must actually work: {body}");

    let (_, me) = server.get_with_cookie("/api/v1/auth/me", cookie.as_deref()).await;
    assert_eq!(me["role"], "admin", "{me}");
}

#[tokio::test]
async fn a_restart_does_not_mint_a_second_administrator() {
    let Some(server) = TestServer::start().await else { return };

    let first = kasl_server::provision::ensure_some_admin(&server.pool, "admin@kasl.local")
        .await
        .expect("the first run")
        .expect("a password");

    // Every boot calls this. If it made an account each time, a restart would
    // print a new password and quietly rotate the credential an operator had
    // written down - or worse, keep working while they think it did not.
    let second = kasl_server::provision::ensure_some_admin(&server.pool, "admin@kasl.local")
        .await
        .expect("a second run should be a no-op");
    assert!(second.is_none(), "a restart must not create another administrator");

    let (status, _, _) = server.login("admin@kasl.local", &first).await;
    assert_eq!(status, StatusCode::OK, "and the first password must still work");
    assert_eq!(server.count("users").await, 2, "the seeded agent's account plus this one");
}

#[tokio::test]
async fn an_existing_administrator_is_left_alone() {
    let Some(server) = TestServer::start().await else { return };

    server.add_admin("boss@example.test", PASSWORD).await;
    let generated = kasl_server::provision::ensure_some_admin(&server.pool, "admin@kasl.local")
        .await
        .expect("the check should succeed");

    // The operator named their own administrator. Adding a second one with a
    // password printed to the console would be a way in that nobody asked for.
    assert!(generated.is_none(), "an installation that has an admin needs no other");
    let (status, _, _) = server.login("admin@kasl.local", "anything at all").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "no such account should exist");
}

#[tokio::test]
async fn generated_passwords_differ_between_installations() {
    let Some(first) = TestServer::start().await else { return };
    let Some(second) = TestServer::start().await else { return };

    let one = kasl_server::provision::ensure_some_admin(&first.pool, "admin@kasl.local")
        .await
        .unwrap()
        .unwrap();
    let two = kasl_server::provision::ensure_some_admin(&second.pool, "admin@kasl.local")
        .await
        .unwrap()
        .unwrap();

    // A constant would be worse than no password at all: every installation in
    // the world would ship with the same admin credential.
    assert_ne!(one, two);
}

#[tokio::test]
async fn promoting_an_existing_employee_to_admin_keeps_their_history() {
    let Some(server) = TestServer::start().await else { return };

    // The realistic case: the person already exists because their agent has
    // been reporting for weeks, and now they need to administer the server.
    let before: uuid::Uuid = server.scalar("SELECT id FROM users WHERE email = 'employee@example.test'").await;

    kasl_server::provision::ensure_admin(&server.pool, "employee@example.test", PASSWORD)
        .await
        .expect("an existing user should be promoted");

    let after: uuid::Uuid = server.scalar("SELECT id FROM users WHERE email = 'employee@example.test'").await;
    assert_eq!(before, after, "the account must be the same row, or its days would be orphaned");
    assert_eq!(server.count("users").await, 1, "not a second account");

    let (status, cookie, _) = server.login("employee@example.test", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = server.get_with_cookie("/api/v1/auth/me", cookie.as_deref()).await;
    assert_eq!(body["role"], "admin");
}

#[test]
fn the_bootstrap_refuses_a_password_too_short_to_be_one() {
    let raw = kasl_server::provision::parse_admin("boss@example.com:secret").expect("a well-formed value parses");
    assert_eq!(raw, Some(("boss@example.com".to_string(), "secret".to_string())));

    // A colon in the password is plausible in a generated one and must survive.
    let raw = kasl_server::provision::parse_admin("boss@example.com:a:b:c").unwrap();
    assert_eq!(raw.unwrap().1, "a:b:c");

    assert!(kasl_server::provision::parse_admin("").unwrap().is_none(), "an absent value is not an error");
    assert!(kasl_server::provision::parse_admin("boss@example.com").is_err(), "no password at all is");
    assert!(kasl_server::provision::parse_admin("boss@example.com:").is_err(), "nor an empty one");
}
