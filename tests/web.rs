//! The web UI as the router sees it.
//!
//! The unit tests in `src/web.rs` prove the handler answers correctly; these
//! prove it is wired in the right order. A fallback that shadowed an API route
//! would answer an agent's upload with an HTML page, and the agent would see a
//! `200` with a body it cannot parse.

mod support;

use axum::http::StatusCode;
use support::TestServer;

#[tokio::test]
async fn the_api_wins_over_the_single_page_app() {
    let Some(server) = TestServer::start().await else { return };

    // A real endpoint, called without credentials. The answer must come from
    // the API - a `401` - and not from the fallback, which would hand back the
    // document with a `200` and make the failure invisible to a client.
    let (status, body) = server.get_with_cookie("/api/v1/privacy", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body["error"].is_string(), "an API path must answer JSON, got {body}");
}

#[tokio::test]
async fn an_unknown_api_path_does_not_fall_through_to_the_app() {
    let Some(server) = TestServer::start().await else { return };

    // Anything under `/api` that does not exist is a client's mistake, and it
    // has to look like one. Serving the app here would tell a kasl agent that
    // a misspelled endpoint worked.
    let (status, body) = server.get_with_cookie("/api/v1/no-such-endpoint", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "an unknown API path must not be answered by the web UI");
    // JSON, like every other API failure: a client parsing our errors should
    // not have to special-case this one.
    assert_eq!(body["error"], "no such endpoint", "{body}");

    // An unversioned path too - a client that forgot the `/v1` is making the
    // same mistake and deserves the same answer.
    let (status, body) = server.get_with_cookie("/api/privacy", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "no such endpoint", "{body}");
}

#[tokio::test]
async fn health_is_still_answered_by_the_server() {
    let Some(server) = TestServer::start().await else { return };

    let (status, body) = server.get_with_cookie("/health", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok", "{body}");
}
