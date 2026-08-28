//! `GET /api/v1/agent/whoami`, against a live database.
//!
//! The endpoint exists so `kasl server connect` can show a person whose token
//! they just pasted. What is under test is therefore not "it answers 200" but
//! that it names the right employee, and that it refuses every token the
//! upload routes refuse - a token that cannot write must not be able to read
//! a name off the installation either.

mod support;

use axum::http::StatusCode;
use support::TestServer;

#[tokio::test]
async fn an_agent_is_told_whose_token_it_holds() {
    let Some(server) = TestServer::start().await else { return };

    server.add_agent("kirill@example.test", "token-kirill").await;

    let (status, body) = server.get_with_header("/api/v1/agent/whoami", Some("Bearer token-kirill")).await;

    assert_eq!(status, StatusCode::OK);
    // The display name, not the address: this is what a person recognises,
    // and the connect summary prints it back to them.
    assert_eq!(body["user_name"], "kirill");
    assert_eq!(body["agent_name"], "seeded");
    assert_eq!(body["api_version"], "v1");
    assert_eq!(body["server_version"], env!("CARGO_PKG_VERSION"));

    server.close().await;
}

#[tokio::test]
async fn two_agents_are_told_apart() {
    let Some(server) = TestServer::start().await else { return };

    server.add_agent("anna@example.test", "token-anna").await;
    server.add_agent("boris@example.test", "token-boris").await;

    let (_, anna) = server.get_with_header("/api/v1/agent/whoami", Some("Bearer token-anna")).await;
    let (_, boris) = server.get_with_header("/api/v1/agent/whoami", Some("Bearer token-boris")).await;

    // The whole point of the endpoint: a token pasted from the wrong window
    // names the wrong person, out loud, while someone is watching.
    assert_eq!(anna["user_name"], "anna");
    assert_eq!(boris["user_name"], "boris");

    server.close().await;
}

#[tokio::test]
async fn an_unknown_token_is_refused() {
    let Some(server) = TestServer::start().await else { return };

    server.add_agent("kirill@example.test", "token-kirill").await;

    let (status, _) = server.get_with_header("/api/v1/agent/whoami", Some("Bearer not-a-real-token")).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);

    server.close().await;
}

#[tokio::test]
async fn a_missing_token_is_refused() {
    let Some(server) = TestServer::start().await else { return };

    let (status, _) = server.get_with_header("/api/v1/agent/whoami", None).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);

    server.close().await;
}

#[tokio::test]
async fn a_revoked_token_is_refused() {
    let Some(server) = TestServer::start().await else { return };

    server.add_agent("kirill@example.test", "token-kirill").await;
    server.execute("UPDATE agents SET revoked_at = now()").await;

    // Revocation has to reach every route the token opens, not just the ones
    // that write. A revoked agent that can still read the employee's name off
    // the server is a revocation that did not finish.
    let (status, _) = server.get_with_header("/api/v1/agent/whoami", Some("Bearer token-kirill")).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);

    server.close().await;
}

#[tokio::test]
async fn a_deactivated_employee_is_refused() {
    let Some(server) = TestServer::start().await else { return };

    server.add_agent("kirill@example.test", "token-kirill").await;
    server.execute("UPDATE users SET active = false").await;

    let (status, _) = server.get_with_header("/api/v1/agent/whoami", Some("Bearer token-kirill")).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);

    server.close().await;
}

#[tokio::test]
async fn a_session_cookie_does_not_open_the_agent_route() {
    let Some(server) = TestServer::start().await else { return };

    server.add_admin("admin@example.test", "correct-horse-battery").await;
    let (_, cookie, _) = server.login("admin@example.test", "correct-horse-battery").await;
    assert!(cookie.is_some(), "the fixture should have signed in");

    // Signed in as a person is not the same as holding an agent token, and
    // this route answers only the second question. Accepting a cookie here
    // would make "whose token is this" answerable without one.
    let (status, _) = server.get_with_cookie("/api/v1/agent/whoami", cookie.as_deref()).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);

    server.close().await;
}
