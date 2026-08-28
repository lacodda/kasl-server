//! Agent authentication: who is allowed to upload, and on whose behalf.
//!
//! An agent presents a bearer token. The server stores only its SHA-256, so a
//! database dump - or a stray log line containing a row - hands out nothing
//! usable. Verification is a hash and a lookup, cheap enough to run on every
//! upload.
//!
//! SHA-256 rather than a password hash on purpose: these tokens are long
//! random strings the server itself issues, not human-chosen secrets, so there
//! is no dictionary to slow an attacker down with. Passwords, when they arrive
//! with the login milestone, need a different treatment.

use axum::{
    Json,
    extract::{FromRequestParts, State},
    http::{StatusCode, request::Parts},
    response::IntoResponse,
};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{app::AppState, error::ApiError};

/// Hex-encoded SHA-256 of a token, which is what `agents.token_hash` holds.
pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    // Hex rather than base64: it survives copying through logs, shells and
    // psql without an encoding argument on either side.
    digest.iter().fold(String::with_capacity(64), |mut acc, byte| {
        use std::fmt::Write;
        let _ = write!(acc, "{byte:02x}");
        acc
    })
}

/// An authenticated agent and the person it reports for.
///
/// Handlers take this as an argument, which makes the check impossible to
/// forget: a route without it simply has no user to write rows for.
#[derive(Debug, Clone, Copy)]
pub struct AuthenticatedAgent {
    pub agent_id: Uuid,
    pub user_id: Uuid,
}

impl FromRequestParts<AppState> for AuthenticatedAgent {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let token = bearer_token(parts)
            .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "missing or malformed Authorization header; expected `Bearer <token>`"))?;

        authenticate(&state.pool, &token).await?.ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "the token is not recognized, has been revoked, or its user is deactivated",
            )
        })
    }
}

/// Extracts the credentials from `Authorization: Bearer <token>`.
fn bearer_token(parts: &Parts) -> Option<String> {
    let header = parts.headers.get(axum::http::header::AUTHORIZATION)?.to_str().ok()?;
    // The scheme is case-insensitive per RFC 7235, and clients differ.
    let (scheme, token) = header.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    (!token.is_empty()).then(|| token.to_string())
}

/// Resolves a token to its agent, refusing revoked agents and inactive users.
///
/// Returns `Ok(None)` when nothing matches: an unknown token and a revoked one
/// are the same answer to whoever is asking, which is the point.
async fn authenticate(pool: &PgPool, token: &str) -> Result<Option<AuthenticatedAgent>, ApiError> {
    let hash = hash_token(token);

    let row: Option<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT a.id, a.user_id FROM agents a
         JOIN users u ON u.id = a.user_id
         WHERE a.token_hash = $1 AND a.revoked_at IS NULL AND u.active",
    )
    .bind(&hash)
    .fetch_optional(pool)
    .await?;

    let Some((agent_id, user_id)) = row else { return Ok(None) };

    // Best-effort liveness stamp: the dashboards use it to spot agents that
    // went quiet. A failure here must not cost the upload its data.
    if let Err(error) = sqlx::query("UPDATE agents SET last_seen_at = now() WHERE id = $1")
        .bind(agent_id)
        .execute(pool)
        .await
    {
        tracing::warn!(%error, %agent_id, "failed to record the agent's last-seen time");
    }

    Ok(Some(AuthenticatedAgent { agent_id, user_id }))
}

/// What an agent is told about its own token: `GET /api/v1/agent/whoami`.
///
/// The only route that answers an agent a question about itself. Without it a
/// token is opaque to the machine holding it: `kasl server connect` could
/// confirm that a token works, but not whose it is, and a token pasted from
/// the wrong chat window would file this machine's days under a colleague's
/// name without a word. The check belongs at connect time, where a person is
/// watching.
#[derive(Debug, serde::Serialize)]
pub struct Whoami {
    /// The employee this agent reports for, as the server displays them.
    pub user_name: String,
    /// The label the administrator gave this agent, typically the machine.
    pub agent_name: String,
    /// The API version this path belongs to. Sent so a client can say "this
    /// server speaks v1" without parsing the URL it just called.
    pub api_version: &'static str,
    /// The server's own version, for the connect summary and for support
    /// questions. Already public on `/health`; repeated here so a connecting
    /// agent needs one round trip, not two.
    pub server_version: &'static str,
}

/// Answers whose token this is.
pub async fn whoami(State(state): State<AppState>, agent: AuthenticatedAgent) -> Result<impl IntoResponse, ApiError> {
    let (user_name, agent_name): (String, String) = sqlx::query_as(
        "SELECT u.display_name, a.name FROM agents a
         JOIN users u ON u.id = a.user_id
         WHERE a.id = $1",
    )
    .bind(agent.agent_id)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(Whoami {
        user_name,
        agent_name,
        api_version: "v1",
        server_version: env!("CARGO_PKG_VERSION"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderValue, Request, header::AUTHORIZATION};

    fn parts_with(header: &str) -> Parts {
        let mut request = Request::new(());
        request.headers_mut().insert(AUTHORIZATION, HeaderValue::from_str(header).unwrap());
        request.into_parts().0
    }

    #[test]
    fn hashing_is_stable_and_hides_the_token() {
        let hash = hash_token("kasl_agent_secret");
        assert_eq!(hash, hash_token("kasl_agent_secret"), "the same token must hash the same way");
        assert_ne!(hash, hash_token("kasl_agent_secre"), "a different token must hash differently");
        assert_eq!(hash.len(), 64, "SHA-256 is 32 bytes, 64 hex characters");
        assert!(!hash.contains("kasl_agent_secret"), "the stored form must not contain the token");
    }

    #[test]
    fn hashing_matches_the_known_sha256_of_a_fixed_input() {
        // Pinned against an external implementation: were the encoding to
        // change, every stored hash would silently stop matching.
        assert_eq!(hash_token("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn reads_the_bearer_scheme_in_any_case() {
        assert_eq!(bearer_token(&parts_with("Bearer tok")).as_deref(), Some("tok"));
        assert_eq!(bearer_token(&parts_with("bearer tok")).as_deref(), Some("tok"));
        assert_eq!(bearer_token(&parts_with("BEARER tok")).as_deref(), Some("tok"));
    }

    #[test]
    fn rejects_headers_that_are_not_a_bearer_token() {
        assert!(
            bearer_token(&parts_with("Basic dXNlcjpwYXNz")).is_none(),
            "another scheme is not ours to interpret"
        );
        assert!(bearer_token(&parts_with("Bearer ")).is_none(), "an empty token is not a token");
        assert!(bearer_token(&parts_with("tok")).is_none(), "a bare value carries no scheme");
    }
}
