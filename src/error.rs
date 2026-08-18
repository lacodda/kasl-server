//! One error shape for the whole API.
//!
//! kasl agents parse what comes back, so a failure has to be as predictable as
//! a success: always JSON, always the same field. A handler returning a bare
//! status leaves the agent guessing whether to retry, fix its payload, or tell
//! the employee something is wrong.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

/// A failed request, with the status the client should act on.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    /// The payload was understood but is not acceptable - the agent must fix
    /// it rather than retry.
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    /// Whose fault this was. A batch reads it to tell a day it should give up
    /// on from one it should send again later.
    pub fn status(&self) -> StatusCode {
        self.status
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        // Server-side faults are logged in full and reported in outline: an
        // agent has no use for a database error, and the message could carry
        // details of someone else's data.
        if self.status.is_server_error() {
            tracing::error!(status = %self.status, error = %self.message, "request failed");
            return (self.status, Json(json!({ "error": "internal server error" }))).into_response();
        }

        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    /// Anything that failed inside the server and has no better status. The
    /// message is logged in full and answered in outline, as with any 500.
    fn from(error: anyhow::Error) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, format!("{error:#}"))
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn body_of(error: ApiError) -> serde_json::Value {
        let response = error.into_response();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn client_errors_explain_themselves() {
        let body = body_of(ApiError::bad_request("date is not a calendar date")).await;
        assert_eq!(body["error"], "date is not a calendar date");
    }

    #[tokio::test]
    async fn server_errors_do_not_leak_their_cause() {
        let body = body_of(ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "relation \"users\" does not exist")).await;
        assert_eq!(body["error"], "internal server error", "internals must not reach the client");
    }

    #[test]
    fn the_status_survives_into_the_response() {
        let response = ApiError::new(StatusCode::UNAUTHORIZED, "nope").into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
