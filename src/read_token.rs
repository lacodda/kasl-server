//! Read tokens: the credential a tool reads the server with - a dashboard the
//! team's hours (ADR 0025), Prometheus the server's own health (ADR 0026).
//!
//! A tool cannot sign in, so it presents a bearer token the way an agent does -
//! but a read token is not an agent's. An agent's writes one person's days; a
//! read token reads what its scope names, with its owner's authority, and
//! nothing else. Three rules shape it:
//!
//! * **It reads with its owner's eyes, as they are now.** The owner's role and
//!   department are read on every request through [`Reader`], so a token
//!   narrows when its owner's authority does and stops when their account is
//!   deactivated - no second list of who a token may see to keep in step.
//! * **Its scope is part of its type.** A route takes `ReadToken<Hours>` or
//!   `ReadToken<Metrics>`, and a token of any other scope is refused before the
//!   handler runs. A route cannot forget to check, because a route that does
//!   not name a scope has no reader at all.
//! * **It is issued by whoever will read with it**, for themselves, if their
//!   role may hold its scope ([`ReadScope::held_by`]); an administrator may
//!   revoke anybody's.

use std::marker::PhantomData;

use axum::{
    Json,
    extract::{FromRequestParts, Path, State},
    http::{StatusCode, request::Parts},
    response::IntoResponse,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    admin::{Reader, generate_token, require_manager_or_admin},
    app::AppState,
    audit,
    auth::{bearer_token, hash_token},
    error::ApiError,
    login::CurrentUser,
    model::UserRole,
};

/// The prefix that tells a read token from an agent's at a glance.
pub const READ_TOKEN_PREFIX: &str = "kasl_read_";

/// What a token may read - the database's `read_scope`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "read_scope", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ReadScope {
    /// The team's hours: everything `/api/bi/v1` answers.
    Hours,
    /// The server's own health: `/metrics`. Nobody's data.
    Metrics,
}

impl ReadScope {
    /// Whether somebody in `role` may hold a token of this scope - asked when
    /// one is issued and again on every request made with it, so a demotion
    /// ends the reading at once.
    ///
    /// The hours are whoever reads the team on the screens; the server's health
    /// describes the installation, which is the administrator's.
    pub fn held_by(self, role: UserRole) -> bool {
        match self {
            ReadScope::Hours => role != UserRole::Employee,
            ReadScope::Metrics => role == UserRole::Admin,
        }
    }
}

/// A scope as a type, for the extractor to check.
pub trait Scope: Send + Sync + 'static {
    const SCOPE: ReadScope;
}

/// The marker for [`ReadScope::Hours`].
pub struct Hours;

impl Scope for Hours {
    const SCOPE: ReadScope = ReadScope::Hours;
}

/// The marker for [`ReadScope::Metrics`].
pub struct Metrics;

impl Scope for Metrics {
    const SCOPE: ReadScope = ReadScope::Metrics;
}

/// A request made with a live read token of scope `S`.
///
/// Taken as a handler argument, like [`crate::auth::AuthenticatedAgent`]: a
/// route without it has nobody to read for.
pub struct ReadToken<S: Scope> {
    pub id: Uuid,
    /// Whose eyes the token reads with: its owner's, as of this request.
    pub reader: Reader,
    scope: PhantomData<fn() -> S>,
}

impl<S: Scope> FromRequestParts<AppState> for ReadToken<S> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let token = bearer_token(parts)
            .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "missing or malformed Authorization header; expected `Bearer <token>`"))?;

        let row: Option<(Uuid, Uuid, UserRole, ReadScope)> = sqlx::query_as(
            "SELECT t.id, t.owner_id, u.role, t.scope FROM read_tokens t
             JOIN users u ON u.id = t.owner_id
             WHERE t.token_hash = $1 AND t.revoked_at IS NULL AND u.active",
        )
        .bind(hash_token(&token))
        .fetch_optional(&state.pool)
        .await?;

        // An unknown token, a revoked one and one whose owner has left are one
        // answer, as they are for an agent's: which of the three it was is
        // nothing the holder needs confirmed.
        let Some((id, owner_id, role, scope)) = row else {
            return Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "the token is not recognized, has been revoked, or its owner is deactivated",
            ));
        };
        if scope != S::SCOPE {
            return Err(ApiError::new(StatusCode::FORBIDDEN, "this token's scope does not cover this"));
        }
        // Issued by a manager who has since become an employee, or by an
        // administrator who has since become a manager: the screens refuse
        // them what the token reads now, and so does the token.
        if !scope.held_by(role) {
            return Err(ApiError::new(StatusCode::FORBIDDEN, "the token's owner may no longer read this"));
        }

        // Best-effort, as an agent's last-seen stamp is: failing to record the
        // moment must not cost the dashboard its answer.
        if let Err(error) = sqlx::query("UPDATE read_tokens SET last_used_at = now() WHERE id = $1")
            .bind(id)
            .execute(&state.pool)
            .await
        {
            tracing::warn!(%error, token_id = %id, "failed to record the read token's last use");
        }

        Ok(Self {
            id,
            reader: Reader { user_id: owner_id, role },
            scope: PhantomData,
        })
    }
}

// Issuing and revoking: `/api/v1/read-tokens` ------------------------------------

/// A token as the screen lists it. Never the token itself.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct TokenRow {
    pub id: Uuid,
    pub name: String,
    pub scope: ReadScope,
    pub owner_id: Uuid,
    /// The owner's display name, so an administrator's list needs no second
    /// request per row.
    pub owner: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct NewToken {
    pub name: String,
    /// What it reads. Required: a default would be the scope nobody chose.
    pub scope: ReadScope,
}

/// What issuing answers: the one time the token is shown.
#[derive(Debug, Serialize)]
pub struct IssuedToken {
    pub id: Uuid,
    pub name: String,
    pub scope: ReadScope,
    pub token: String,
    pub notice: &'static str,
}

/// `GET /api/v1/read-tokens` - the caller's tokens, or every token to an
/// administrator, live ones first.
pub async fn list(State(state): State<AppState>, user: CurrentUser) -> Result<impl IntoResponse, ApiError> {
    require_manager_or_admin(&user)?;

    let tokens: Vec<TokenRow> = sqlx::query_as(
        "SELECT t.id, t.name, t.scope, t.owner_id, u.display_name AS owner, t.created_at, t.last_used_at, t.revoked_at
         FROM read_tokens t
         JOIN users u ON u.id = t.owner_id
         WHERE $1 OR t.owner_id = $2
         ORDER BY t.revoked_at IS NOT NULL, t.created_at DESC",
    )
    .bind(user.role == UserRole::Admin)
    .bind(user.user_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(tokens))
}

/// `POST /api/v1/read-tokens` - issues the caller a token of their own.
pub async fn issue(State(state): State<AppState>, user: CurrentUser, Json(new): Json<NewToken>) -> Result<impl IntoResponse, ApiError> {
    require_manager_or_admin(&user)?;

    let name = new.name.trim();
    if name.is_empty() {
        return Err(ApiError::bad_request("a token needs a name; what will read with it is the usual one"));
    }
    let scope = new.scope;
    if !scope.held_by(user.role) {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "your role may not hold a token of this scope"));
    }

    let token = generate_token(READ_TOKEN_PREFIX);
    let id: Uuid = sqlx::query_scalar("INSERT INTO read_tokens (owner_id, name, scope, token_hash) VALUES ($1, $2, $3, $4) RETURNING id")
        .bind(user.user_id)
        .bind(name)
        .bind(scope)
        .bind(hash_token(&token))
        .fetch_one(&state.pool)
        .await?;

    tracing::info!(%id, by = %user.user_id, "issued a read token");
    // The entry that stands for every read the token will make: a dashboard
    // polls, and a line per poll would bury the log (ADR 0025). The token
    // itself is never recorded.
    audit::Entry::new(audit::action::READ_TOKEN_ISSUED)
        .by(user.user_id)
        .by_email(&user.email)
        .on(id)
        .labelled(name)
        .with(serde_json::json!({"scope": scope}))
        .record(&state.pool)
        .await;

    Ok((
        StatusCode::CREATED,
        Json(IssuedToken {
            id,
            name: name.to_string(),
            scope,
            token,
            notice: "this token is shown once; the server keeps only its hash",
        }),
    ))
}

/// `DELETE /api/v1/read-tokens/{id}` - by its owner, or by an administrator.
///
/// Idempotent the way revoking an agent is: a second revoke changes nothing,
/// and does not move the moment access ended.
pub async fn revoke(State(state): State<AppState>, user: CurrentUser, Path(id): Path<Uuid>) -> Result<impl IntoResponse, ApiError> {
    require_manager_or_admin(&user)?;
    let is_admin = user.role == UserRole::Admin;

    let revoked: Option<String> =
        sqlx::query_scalar("UPDATE read_tokens SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL AND ($2 OR owner_id = $3) RETURNING name")
            .bind(id)
            .bind(is_admin)
            .bind(user.user_id)
            .fetch_optional(&state.pool)
            .await?;

    if revoked.is_none() {
        // Already revoked, or not this caller's to revoke. Somebody else's
        // token is answered as one that does not exist: whose tokens exist is
        // not a manager's to learn by probing ids.
        let theirs: Option<Uuid> = sqlx::query_scalar("SELECT id FROM read_tokens WHERE id = $1 AND ($2 OR owner_id = $3)")
            .bind(id)
            .bind(is_admin)
            .bind(user.user_id)
            .fetch_optional(&state.pool)
            .await?;
        if theirs.is_none() {
            return Err(ApiError::new(StatusCode::NOT_FOUND, "no such token"));
        }
    }

    tracing::info!(token_id = %id, by = %user.user_id, "revoked a read token");
    let mut entry = audit::Entry::new(audit::action::READ_TOKEN_REVOKED)
        .by(user.user_id)
        .by_email(&user.email)
        .on(id)
        .with(serde_json::json!({"already_revoked": revoked.is_none()}));
    if let Some(name) = &revoked {
        entry = entry.labelled(name);
    }
    entry.record(&state.pool).await;

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_token_says_what_it_is_and_is_no_agent_token() {
        let token = generate_token(READ_TOKEN_PREFIX);
        assert!(token.starts_with("kasl_read_"), "{token}");
        assert_eq!(token.len(), READ_TOKEN_PREFIX.len() + 64, "32 bytes as hex");
    }

    #[test]
    fn the_scope_is_snake_case_on_the_wire() {
        assert_eq!(serde_json::to_string(&ReadScope::Hours).unwrap(), "\"hours\"");
        assert_eq!(serde_json::to_string(&ReadScope::Metrics).unwrap(), "\"metrics\"");
    }

    #[test]
    fn the_hours_are_the_team_s_readers_and_the_metrics_the_administrator_s() {
        assert!(!ReadScope::Hours.held_by(UserRole::Employee));
        assert!(ReadScope::Hours.held_by(UserRole::Manager));
        assert!(ReadScope::Hours.held_by(UserRole::Admin));

        assert!(!ReadScope::Metrics.held_by(UserRole::Employee));
        assert!(!ReadScope::Metrics.held_by(UserRole::Manager));
        assert!(ReadScope::Metrics.held_by(UserRole::Admin));
    }
}
