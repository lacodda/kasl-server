//! Passwords and browser sessions.
//!
//! Two kinds of credential live on this server and they are deliberately not
//! the same thing. An agent presents a long random token the server issued,
//! hashed with SHA-256 because there is no dictionary to slow anyone down with
//! (see [`crate::auth`]). A person types a password they chose, which is
//! guessable at scale, so it gets Argon2id and a per-password salt.
//!
//! Sessions are server-side. A signed self-contained token would save a query
//! per request and cost the one thing this server cannot give up: the ability
//! to end someone's access now, on the afternoon they leave (ADR 0007).

use anyhow::{Context, Result};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::hash_token;

/// How long a session lives without being used.
///
/// A working fortnight: long enough that nobody logs in twice a day, short
/// enough that a forgotten laptop stops being a way in.
pub const SESSION_LIFETIME_DAYS: i64 = 14;

/// The cookie the browser carries. Named for the product so it is obvious in a
/// developer console which server put it there.
pub const SESSION_COOKIE: &str = "kasl_session";

/// Hashes a password for storage.
pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| anyhow::anyhow!("failed to hash the password: {error}"))
}

/// Checks a password against a stored hash.
///
/// Any failure is `false`, never an error: a malformed hash in the database and
/// a wrong password are the same answer to whoever is asking, and telling them
/// apart is information the caller has no business acting on differently.
pub fn verify_password(password: &str, stored: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(stored) else {
        tracing::error!("a stored password hash could not be parsed; the account cannot be logged into");
        return false;
    };

    Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok()
}

/// A new session token: what the browser gets, and what the database stores.
pub struct IssuedSession {
    /// Handed to the client once, in a cookie, and never stored anywhere.
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

/// Creates a session for a user.
pub async fn issue(pool: &PgPool, user_id: Uuid) -> Result<IssuedSession> {
    use rand::RngExt;

    // 32 bytes from the OS: the token is the entire credential, so it has to be
    // unguessable rather than merely unique.
    let bytes: [u8; 32] = rand::rng().random();
    let token = bytes.iter().fold(String::with_capacity(64), |mut acc, byte| {
        use std::fmt::Write;
        let _ = write!(acc, "{byte:02x}");
        acc
    });
    let expires_at = Utc::now() + Duration::days(SESSION_LIFETIME_DAYS);

    sqlx::query("INSERT INTO sessions (user_id, token_hash, expires_at) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(hash_token(&token))
        .bind(expires_at)
        .execute(pool)
        .await
        .context("failed to store the session")?;

    Ok(IssuedSession { token, expires_at })
}

/// Who a session token belongs to, if it is still good for anything.
#[derive(Debug, Clone)]
pub struct SessionUser {
    pub session_id: Uuid,
    pub user_id: Uuid,
    pub role: crate::model::UserRole,
    /// Read alongside the rest so an audit entry can name the actor without a
    /// second query per recorded action.
    pub email: String,
}

/// Resolves a token to its user, refusing expired sessions and inactive people.
///
/// The expiry is checked in the query rather than in Rust: a row that outlived
/// its welcome must not authenticate anyone even if the sweep that deletes it
/// has not run.
pub async fn authenticate(pool: &PgPool, token: &str) -> Result<Option<SessionUser>> {
    let row: Option<(Uuid, Uuid, crate::model::UserRole, String)> = sqlx::query_as(
        "SELECT s.id, s.user_id, u.role, u.email FROM sessions s
         JOIN users u ON u.id = s.user_id
         WHERE s.token_hash = $1 AND s.expires_at > now() AND u.active",
    )
    .bind(hash_token(token))
    .fetch_optional(pool)
    .await?;

    let Some((session_id, user_id, role, email)) = row else { return Ok(None) };

    // Rolling expiry, best-effort: someone working through the day should not
    // be logged out mid-afternoon, and failing to extend costs them nothing
    // worse than logging in again.
    if let Err(error) = sqlx::query("UPDATE sessions SET last_used_at = now(), expires_at = now() + ($2 || ' days')::interval WHERE id = $1")
        .bind(session_id)
        .bind(SESSION_LIFETIME_DAYS.to_string())
        .execute(pool)
        .await
    {
        tracing::warn!(%error, %session_id, "failed to extend the session");
    }

    Ok(Some(SessionUser {
        session_id,
        user_id,
        role,
        email,
    }))
}

/// Ends one session - what "log out" does.
pub async fn revoke(pool: &PgPool, session_id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM sessions WHERE id = $1").bind(session_id).execute(pool).await?;
    Ok(())
}

/// Ends every session a user has - what "log out everywhere" does, and what
/// deactivating an employee should be followed by.
pub async fn revoke_all(pool: &PgPool, user_id: Uuid) -> Result<u64> {
    let deleted = sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(deleted)
}

/// Deletes sessions that have expired.
///
/// Not required for correctness - `authenticate` already refuses them - but a
/// table that only grows is a table nobody wants to meet in a year.
pub async fn sweep_expired(pool: &PgPool) -> Result<u64> {
    let deleted = sqlx::query("DELETE FROM sessions WHERE expires_at <= now()")
        .execute(pool)
        .await?
        .rows_affected();
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_verifies_against_its_own_hash_and_nothing_else() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(verify_password("correct horse battery staple", &hash));
        assert!(!verify_password("Correct horse battery staple", &hash), "verification is exact");
        assert!(!verify_password("", &hash));
    }

    #[test]
    fn the_stored_form_reveals_nothing() {
        let hash = hash_password("hunter2").unwrap();
        assert!(!hash.contains("hunter2"), "the password must not survive in the hash");
        assert!(hash.starts_with("$argon2id$"), "a memory-hard hash, not a bare digest: {hash}");
    }

    #[test]
    fn the_same_password_hashes_differently_every_time() {
        // The salt is what makes two employees who chose the same password
        // indistinguishable in a database dump.
        let first = hash_password("same").unwrap();
        let second = hash_password("same").unwrap();
        assert_ne!(first, second);
        assert!(verify_password("same", &first) && verify_password("same", &second));
    }

    #[test]
    fn a_damaged_hash_refuses_rather_than_admits() {
        // Truncation, a stray edit in psql, a half-written migration: none of it
        // may become a way in.
        assert!(!verify_password("anything", ""));
        assert!(!verify_password("anything", "not-a-hash"));
        assert!(!verify_password("anything", "$argon2id$v=19$m=19456,t=2,p=1$truncated"));
    }
}
