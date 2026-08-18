//! The login endpoints and the extractor that guards everything behind them.
//!
//! Only the browser goes through here. kasl agents keep presenting their bearer
//! token to the ingest routes and are unaffected by any of this - a working
//! agent must keep working across a server upgrade (ADR 0004).

use axum::{
    Json,
    extract::{FromRequestParts, State},
    http::{StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    app::AppState,
    error::ApiError,
    model::UserRole,
    session::{self, SESSION_COOKIE},
};

#[derive(Debug, Deserialize)]
pub struct Credentials {
    pub email: String,
    pub password: String,
}

/// Who the caller is, as the UI needs to know it.
#[derive(Debug, Serialize)]
pub struct Identity {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub role: UserRole,
}

/// An authenticated person, taken as a handler argument.
///
/// A route that omits it has no user to act for, which makes forgetting the
/// check a compile error rather than a security hole.
#[derive(Debug, Clone, Copy)]
pub struct CurrentUser {
    pub session_id: Uuid,
    pub user_id: Uuid,
    pub role: UserRole,
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let token = session_cookie(parts).ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "not signed in"))?;

        let user = session::authenticate(&state.pool, &token)
            .await
            .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
            .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "the session has expired or been ended"))?;

        Ok(Self {
            session_id: user.session_id,
            user_id: user.user_id,
            role: user.role,
        })
    }
}

impl CurrentUser {
    /// Refuses anyone who is not an administrator.
    ///
    /// Roles get their own milestone; this is the one check that cannot wait,
    /// because the account-management routes arrive with it.
    pub fn require_admin(&self) -> Result<(), ApiError> {
        if self.role == UserRole::Admin {
            return Ok(());
        }
        // Deliberately not "you are not an admin": whether a route exists is
        // not something a signed-in employee needs confirmed.
        Err(ApiError::new(StatusCode::FORBIDDEN, "not allowed"))
    }
}

/// Reads our cookie out of the `Cookie` header.
fn session_cookie(parts: &Parts) -> Option<String> {
    let header = parts.headers.get(header::COOKIE)?.to_str().ok()?;
    header.split(';').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name.trim() == SESSION_COOKIE).then(|| value.trim().to_string())
    })
}

/// Signs in, or refuses without saying which half was wrong.
pub async fn login(State(state): State<AppState>, Json(credentials): Json<Credentials>) -> Result<Response, ApiError> {
    let row: Option<(Uuid, Option<String>)> = sqlx::query_as("SELECT id, password_hash FROM users WHERE lower(email) = lower($1) AND active")
        .bind(&credentials.email)
        .fetch_optional(&state.pool)
        .await?;

    // An unknown email, a deactivated account and a wrong password are one
    // answer. Distinguishing them would turn the login form into a way to
    // enumerate who works here.
    let refused = || ApiError::new(StatusCode::UNAUTHORIZED, "wrong email or password");

    let Some((user_id, Some(hash))) = row else {
        // An account with no password set cannot be logged into - and the work
        // to verify a password is done anyway, so that "no such user" and
        // "wrong password" do not differ by a measurable pause.
        session::verify_password(&credentials.password, DUMMY_HASH);
        return Err(refused());
    };

    if !session::verify_password(&credentials.password, &hash) {
        return Err(refused());
    }

    let issued = session::issue(&state.pool, user_id).await?;
    tracing::info!(%user_id, "signed in");

    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, cookie_for(&issued.token, state.secure_cookies))],
        Json(serde_json::json!({"status": "ok"})),
    )
        .into_response())
}

/// Signs out of this session only.
pub async fn logout(State(state): State<AppState>, user: CurrentUser) -> Result<Response, ApiError> {
    session::revoke(&state.pool, user.session_id).await?;
    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, expired_cookie(state.secure_cookies))],
        Json(serde_json::json!({"status": "ok"})),
    )
        .into_response())
}

/// Signs out everywhere - the answer to a laptop left on a train.
pub async fn logout_everywhere(State(state): State<AppState>, user: CurrentUser) -> Result<Response, ApiError> {
    let ended = session::revoke_all(&state.pool, user.user_id).await?;
    tracing::info!(user_id = %user.user_id, ended, "ended every session");
    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, expired_cookie(state.secure_cookies))],
        Json(serde_json::json!({"status": "ok", "ended": ended})),
    )
        .into_response())
}

/// Who am I - what the SPA calls on load to decide whether to show the login.
pub async fn me(State(state): State<AppState>, user: CurrentUser) -> Result<impl IntoResponse, ApiError> {
    let (email, display_name): (String, String) = sqlx::query_as("SELECT email, display_name FROM users WHERE id = $1")
        .bind(user.user_id)
        .fetch_one(&state.pool)
        .await?;

    Ok(Json(Identity {
        id: user.user_id,
        email,
        display_name,
        role: user.role,
    }))
}

/// Builds the session cookie.
///
/// `HttpOnly` so no script can read the token even if one is injected;
/// `SameSite=Strict` because every caller is our own page on our own origin,
/// which also makes CSRF tokens unnecessary; `Secure` unless the operator is on
/// plain HTTP, where an unconditional flag would silently break every login.
fn cookie_for(token: &str, secure: bool) -> String {
    let max_age = session::SESSION_LIFETIME_DAYS * 24 * 60 * 60;
    let secure = if secure { "; Secure" } else { "" };
    format!("{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}{secure}")
}

/// The same cookie, already expired: what tells the browser to forget it.
fn expired_cookie(secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{secure}")
}

/// A real Argon2 hash of a value nobody knows, verified against when the email
/// is unknown so that the refusal costs the same either way.
const DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHR2YWx1ZQ$K7gNU3sdo+OL0wNhqoVWhr3g6s1xYv72ol/pe/Unols";

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderValue, Request, header::COOKIE};

    fn parts_with(cookie: &str) -> Parts {
        let mut request = Request::new(());
        request.headers_mut().insert(COOKIE, HeaderValue::from_str(cookie).unwrap());
        request.into_parts().0
    }

    #[test]
    fn finds_our_cookie_among_the_others() {
        assert_eq!(session_cookie(&parts_with("kasl_session=abc")).as_deref(), Some("abc"));
        assert_eq!(
            session_cookie(&parts_with("theme=dark; kasl_session=abc; lang=en")).as_deref(),
            Some("abc"),
            "a browser sends everything it has for the origin"
        );
        assert_eq!(session_cookie(&parts_with("kasl_session=abc ")).as_deref(), Some("abc"));
    }

    #[test]
    fn ignores_cookies_that_are_not_ours() {
        assert!(session_cookie(&parts_with("theme=dark")).is_none());
        // A prefix match would accept this one, and it is not our cookie.
        assert!(session_cookie(&parts_with("kasl_session_other=abc")).is_none());
    }

    #[test]
    fn the_cookie_cannot_be_read_by_script_or_sent_across_sites() {
        let cookie = cookie_for("token-value", true);
        assert!(cookie.contains("HttpOnly"), "a readable token is a stealable token: {cookie}");
        assert!(cookie.contains("SameSite=Strict"), "{cookie}");
        assert!(cookie.contains("Secure"), "{cookie}");
        assert!(cookie.contains("Max-Age=1209600"), "fourteen days in seconds: {cookie}");
    }

    #[test]
    fn plain_http_gets_a_cookie_without_secure() {
        // A Secure cookie on http:// is silently dropped by the browser, which
        // looks exactly like "login does nothing".
        let cookie = cookie_for("token-value", false);
        assert!(!cookie.contains("Secure"), "{cookie}");
        assert!(cookie.contains("HttpOnly"), "the rest of the protection stays: {cookie}");
    }

    #[test]
    fn logging_out_clears_the_cookie() {
        let cookie = expired_cookie(true);
        assert!(cookie.contains("Max-Age=0"), "{cookie}");
        assert!(cookie.starts_with("kasl_session=;"), "with no value left behind: {cookie}");
    }

    #[test]
    fn the_dummy_hash_is_a_real_hash_that_matches_nothing() {
        // If this stopped parsing, the unknown-email path would return early
        // from a parse error instead of doing the work it exists to do.
        assert!(!crate::session::verify_password("", DUMMY_HASH));
        assert!(!crate::session::verify_password("password", DUMMY_HASH));
    }

    #[test]
    fn only_an_admin_passes_the_admin_check() {
        let user = |role| CurrentUser {
            session_id: Uuid::nil(),
            user_id: Uuid::nil(),
            role,
        };
        assert!(user(UserRole::Admin).require_admin().is_ok());
        assert!(user(UserRole::Manager).require_admin().is_err());
        assert!(user(UserRole::Employee).require_admin().is_err());
        assert_eq!(
            user(UserRole::Employee).require_admin().unwrap_err().to_string(),
            "not allowed",
            "the refusal must not confirm what the route is"
        );
    }
}
