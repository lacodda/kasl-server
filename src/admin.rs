//! Managing people and the agents that report for them.
//!
//! Everything here is behind a session (ADR 0007) and a role. Two rules shape
//! the routes, both settled before they were written (ADR 0008):
//!
//! * **A manager reads, an administrator writes.** Until departments exist
//!   there is nothing to scope a manager's authority to, and handing out the
//!   power to issue agent tokens company-wide - with no audit log yet to notice
//!   - is not a default worth shipping.
//! * **A token is shown once.** The server keeps its SHA-256 and nothing else,
//!   so an issued token that was not written down is replaced, not recovered.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{app::AppState, audit, auth::hash_token, error::ApiError, login::CurrentUser, model::UserRole, session};

/// The shortest password the server will store.
///
/// A floor, not a policy: complexity rules belong where they can be explained
/// to the person typing, and a server that refuses `hunter2` while accepting
/// `Passw0rd!` has chosen theatre over arithmetic.
const MIN_PASSWORD_LENGTH: usize = 8;

/// A person as the admin screens list them.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct UserRow {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub role: UserRole,
    pub active: bool,
    /// Whether they can sign in at all. Accounts created for an agent have no
    /// password, and the UI needs to show that rather than imply a login that
    /// does not exist.
    pub has_password: bool,
    /// Agents currently able to report for them.
    pub agents: i64,
    /// The most recent moment any of their agents was heard from.
    pub last_seen_at: Option<DateTime<Utc>>,
    pub department_id: Option<Uuid>,
    /// Carried alongside the id so a list can be rendered without a second
    /// request per row.
    pub department: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct NewUser {
    pub email: String,
    pub display_name: Option<String>,
    #[serde(default = "default_role")]
    pub role: UserRole,
    /// Initial password. Optional: an account that only owns an agent's data
    /// has no reason to be signed into.
    pub password: Option<String>,
}

fn default_role() -> UserRole {
    UserRole::Employee
}

/// A change to an existing person. Every field is optional; absent means
/// "leave it alone" rather than "clear it".
#[derive(Debug, Deserialize)]
pub struct UserPatch {
    pub display_name: Option<String>,
    pub role: Option<UserRole>,
    pub active: Option<bool>,
    /// Sets or replaces the password. There is no way to remove one here: an
    /// account that could be signed into yesterday and silently cannot today is
    /// a support call, not a feature.
    pub password: Option<String>,
}

/// An agent as the admin screens list it. Never the token.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AgentRow {
    pub id: Uuid,
    pub name: String,
    pub revoked_at: Option<DateTime<Utc>>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct NewAgent {
    /// Human label, typically the machine the agent runs on.
    pub name: String,
}

/// The one response that carries a token, and the only time it exists.
#[derive(Debug, Serialize)]
pub struct IssuedAgent {
    pub id: Uuid,
    pub name: String,
    pub token: String,
    /// Said plainly in the payload, because the UI that forgets to say it is
    /// the reason someone loses a token they never wrote down.
    pub notice: &'static str,
}

/// Lists everyone. The one route a manager may call.
/// Lists the people the caller may see.
///
/// An administrator sees everyone. A manager sees the departments they run,
/// plus themselves - a manager who could not find their own row would think
/// the page was broken. Someone with no department is visible to the admin
/// alone: an unfiled person is noticed at once because they are missing from
/// every manager's list, whereas showing them to every manager would be a leak
/// nobody sees happening (ADR 0009).
pub async fn list_users(State(state): State<AppState>, user: CurrentUser) -> Result<impl IntoResponse, ApiError> {
    require_manager_or_admin(&user)?;

    let users: Vec<UserRow> = sqlx::query_as(
        "SELECT u.id, u.email, u.display_name, u.role, u.active,
                (u.password_hash IS NOT NULL) AS has_password,
                (SELECT count(*) FROM agents a WHERE a.user_id = u.id AND a.revoked_at IS NULL) AS agents,
                (SELECT max(a.last_seen_at) FROM agents a WHERE a.user_id = u.id) AS last_seen_at,
                u.department_id,
                d.name AS department,
                u.created_at
         FROM users u
         LEFT JOIN departments d ON d.id = u.department_id
         WHERE $1
            OR u.id = $2
            OR u.department_id IN (SELECT id FROM departments WHERE manager_id = $2)
         ORDER BY u.display_name, u.email",
    )
    .bind(user.role == UserRole::Admin)
    .bind(user.user_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(users))
}

/// Creates a person.
pub async fn create_user(State(state): State<AppState>, user: CurrentUser, Json(new): Json<NewUser>) -> Result<impl IntoResponse, ApiError> {
    user.require_admin()?;

    let email = new.email.trim();
    if !looks_like_an_email(email) {
        return Err(ApiError::bad_request("that does not look like an email address"));
    }
    let password_hash = match new.password.as_deref() {
        Some(password) => Some(hash_new_password(password)?),
        None => None,
    };
    // The local part until someone sets a real one - the same default the
    // environment-seeded accounts get, so the two look alike in a list.
    let display_name = new
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| email.split('@').next().unwrap_or(email));

    let created: Result<Uuid, sqlx::Error> =
        sqlx::query_scalar("INSERT INTO users (email, display_name, role, password_hash) VALUES ($1, $2, $3, $4) RETURNING id")
            .bind(email)
            .bind(display_name)
            .bind(new.role)
            .bind(password_hash.as_deref())
            .fetch_one(&state.pool)
            .await;

    let id = match created {
        Ok(id) => id,
        // The unique index on lower(email). Answered as a conflict rather than
        // a 500: the admin typed an address that is already here, which is
        // something they can act on.
        Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
            return Err(ApiError::new(StatusCode::CONFLICT, "someone with that email address already exists"));
        }
        Err(error) => return Err(error.into()),
    };

    tracing::info!(%id, by = %user.user_id, "created a user");
    audit::Entry::new(audit::action::USER_CREATED)
        .by(user.user_id)
        .by_email(&user.email)
        .on(id)
        .labelled(email)
        .with(serde_json::json!({"role": new.role, "with_password": password_hash.is_some()}))
        .record(&state.pool)
        .await;

    Ok((StatusCode::CREATED, Json(serde_json::json!({"id": id}))))
}

/// Changes a person: their name, role, password, or whether they are active.
pub async fn update_user(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(target): Path<Uuid>,
    Json(patch): Json<UserPatch>,
) -> Result<impl IntoResponse, ApiError> {
    user.require_admin()?;

    // The last administrator cannot be demoted or deactivated. Both would leave
    // an installation nobody can administer, recoverable only by running the
    // `admin` subcommand on the host - which is not where an admin who just
    // clicked a toggle in a browser will think to look.
    let losing_admin = patch.role.is_some_and(|role| role != UserRole::Admin) || patch.active == Some(false);
    if losing_admin && is_last_admin(&state.pool, target).await? {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "this is the only administrator; promote someone else first",
        ));
    }

    let password_hash = match patch.password.as_deref() {
        Some(password) => Some(hash_new_password(password)?),
        None => None,
    };

    let updated = sqlx::query(
        "UPDATE users SET
             display_name = coalesce($2, display_name),
             role = coalesce($3, role),
             active = coalesce($4, active),
             password_hash = coalesce($5, password_hash)
         WHERE id = $1",
    )
    .bind(target)
    .bind(patch.display_name.as_deref().map(str::trim).filter(|name| !name.is_empty()))
    .bind(patch.role)
    .bind(patch.active)
    .bind(password_hash.as_deref())
    .execute(&state.pool)
    .await?
    .rows_affected();

    if updated == 0 {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "no such user"));
    }

    // Deactivation and a password change both mean the old sessions should not
    // survive: one is someone leaving, the other is usually a suspicion that
    // someone else has been signing in.
    if patch.active == Some(false) || patch.password.is_some() {
        let ended = session::revoke_all(&state.pool, target).await?;
        tracing::info!(%target, ended, "ended sessions after a change to the account");
    }

    tracing::info!(%target, by = %user.user_id, "updated a user");
    // The fields that were touched, never their values: a password reset is
    // worth recording, the password is not.
    audit::Entry::new(audit::action::USER_UPDATED)
        .by(user.user_id)
        .by_email(&user.email)
        .on(target)
        .with(serde_json::json!({
            "display_name": patch.display_name.is_some(),
            "role": patch.role,
            "active": patch.active,
            "password_reset": patch.password.is_some(),
        }))
        .record(&state.pool)
        .await;

    Ok(StatusCode::NO_CONTENT)
}

/// Lists someone's agents, revoked ones included - a withdrawn token is part of
/// the record of what happened.
pub async fn list_agents(State(state): State<AppState>, user: CurrentUser, Path(target): Path<Uuid>) -> Result<impl IntoResponse, ApiError> {
    require_manager_or_admin(&user)?;

    let agents: Vec<AgentRow> = sqlx::query_as("SELECT id, name, revoked_at, last_seen_at, created_at FROM agents WHERE user_id = $1 ORDER BY created_at")
        .bind(target)
        .fetch_all(&state.pool)
        .await?;

    Ok(Json(agents))
}

/// Issues an agent token, shown once.
pub async fn create_agent(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(target): Path<Uuid>,
    Json(new): Json<NewAgent>,
) -> Result<impl IntoResponse, ApiError> {
    user.require_admin()?;

    let name = new.name.trim();
    if name.is_empty() {
        return Err(ApiError::bad_request("an agent needs a name; the machine it runs on is the usual one"));
    }

    // An agent for a deactivated person would be refused on its first upload
    // anyway; saying so here saves someone installing kasl on a laptop to find
    // out.
    let active: Option<bool> = sqlx::query_scalar("SELECT active FROM users WHERE id = $1")
        .bind(target)
        .fetch_optional(&state.pool)
        .await?;
    match active {
        None => return Err(ApiError::new(StatusCode::NOT_FOUND, "no such user")),
        Some(false) => return Err(ApiError::new(StatusCode::CONFLICT, "that account is deactivated")),
        Some(true) => {}
    }

    let token = generate_token();
    let id: Uuid = sqlx::query_scalar("INSERT INTO agents (user_id, name, token_hash) VALUES ($1, $2, $3) RETURNING id")
        .bind(target)
        .bind(name)
        .bind(hash_token(&token))
        .fetch_one(&state.pool)
        .await?;

    tracing::info!(%id, %target, by = %user.user_id, "issued an agent token");
    // The token itself is never recorded - this table is read in a UI and
    // pasted into tickets.
    audit::Entry::new(audit::action::AGENT_ISSUED)
        .by(user.user_id)
        .by_email(&user.email)
        .on(id)
        .labelled(name)
        .with(serde_json::json!({"user_id": target}))
        .record(&state.pool)
        .await;

    Ok((
        StatusCode::CREATED,
        Json(IssuedAgent {
            id,
            name: name.to_string(),
            token,
            notice: "this token is shown once; the server keeps only its hash",
        }),
    ))
}

/// Withdraws an agent's token. The row stays, so its uploads keep an owner.
pub async fn revoke_agent(State(state): State<AppState>, user: CurrentUser, Path(agent): Path<Uuid>) -> Result<impl IntoResponse, ApiError> {
    user.require_admin()?;

    // `revoked_at IS NULL` in the filter makes this idempotent without pretending
    // it succeeded twice: revoking an already-revoked agent must not move the
    // timestamp of when access actually ended.
    let revoked = sqlx::query("UPDATE agents SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL")
        .bind(agent)
        .execute(&state.pool)
        .await?
        .rows_affected();

    if revoked == 0 {
        // Either it does not exist or it was already revoked; both mean the
        // token does not work, which is what the caller wanted.
        let exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM agents WHERE id = $1")
            .bind(agent)
            .fetch_optional(&state.pool)
            .await?;
        if exists.is_none() {
            return Err(ApiError::new(StatusCode::NOT_FOUND, "no such agent"));
        }
    }

    tracing::info!(%agent, by = %user.user_id, "revoked an agent token");
    audit::Entry::new(audit::action::AGENT_REVOKED)
        .by(user.user_id)
        .by_email(&user.email)
        .on(agent)
        .with(serde_json::json!({"already_revoked": revoked == 0}))
        .record(&state.pool)
        .await;

    Ok(StatusCode::NO_CONTENT)
}

/// Changes one's own password.
///
/// Not an admin route: this is how someone stops the admin who set their
/// initial password from knowing it.
#[derive(Debug, Deserialize)]
pub struct PasswordChange {
    pub current: String,
    pub new: String,
}

pub async fn change_own_password(State(state): State<AppState>, user: CurrentUser, Json(change): Json<PasswordChange>) -> Result<impl IntoResponse, ApiError> {
    let stored: Option<String> = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
        .bind(user.user_id)
        .fetch_one(&state.pool)
        .await?;

    // Proving the current password is what stops a borrowed unlocked laptop
    // from becoming a permanent one.
    let Some(stored) = stored else {
        return Err(ApiError::new(StatusCode::CONFLICT, "this account has no password to change"));
    };
    if !session::verify_password(&change.current, &stored) {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "the current password is wrong"));
    }

    let hash = hash_new_password(&change.new)?;
    sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
        .bind(&hash)
        .bind(user.user_id)
        .execute(&state.pool)
        .await?;

    // Every other session ends, and this one survives: changing a password is
    // how someone reacts to a suspicion, and being logged out of the browser
    // they just did it in would be a poor reward.
    sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND id <> $2")
        .bind(user.user_id)
        .bind(user.session_id)
        .execute(&state.pool)
        .await?;

    tracing::info!(user_id = %user.user_id, "changed their password");
    audit::Entry::new(audit::action::PASSWORD_CHANGED)
        .by(user.user_id)
        .by_email(&user.email)
        .on(user.user_id)
        .record(&state.pool)
        .await;

    Ok(StatusCode::NO_CONTENT)
}

/// Both roles that may read the team.
fn require_manager_or_admin(user: &CurrentUser) -> Result<(), ApiError> {
    match user.role {
        UserRole::Admin | UserRole::Manager => Ok(()),
        UserRole::Employee => Err(ApiError::new(StatusCode::FORBIDDEN, "not allowed")),
    }
}

/// Hashes a password after checking it is long enough to be one.
fn hash_new_password(password: &str) -> Result<String, ApiError> {
    if password.chars().count() < MIN_PASSWORD_LENGTH {
        return Err(ApiError::bad_request(format!("the password must be at least {MIN_PASSWORD_LENGTH} characters")));
    }
    session::hash_password(password).map_err(Into::into)
}

/// A new agent token: 32 bytes of entropy, hex.
///
/// Prefixed so that a token found in a log or a config file is recognisable as
/// one, and so a leaked-secret scanner has something to match on.
fn generate_token() -> String {
    use rand::RngExt;

    let bytes: [u8; 32] = rand::rng().random();
    bytes.iter().fold(String::from("kasl_"), |mut acc, byte| {
        use std::fmt::Write;
        let _ = write!(acc, "{byte:02x}");
        acc
    })
}

/// The shallowest possible check: an `@` with something either side.
///
/// Deliberately not a full grammar. The addresses here are typed by an admin
/// who knows their own team, and a regex strict enough to be interesting is
/// strict enough to reject somebody's real address.
fn looks_like_an_email(candidate: &str) -> bool {
    match candidate.split_once('@') {
        Some((local, domain)) => !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.'),
        None => false,
    }
}

/// Whether this user is the only administrator left standing.
async fn is_last_admin(pool: &sqlx::PgPool, target: Uuid) -> Result<bool, ApiError> {
    let others: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE role = 'admin' AND active AND id <> $1")
        .bind(target)
        .fetch_one(pool)
        .await?;

    // Only matters if the target is an active admin themselves; demoting an
    // employee while zero admins exist is a different problem and not this
    // check's business.
    let is_admin: Option<bool> = sqlx::query_scalar("SELECT (role = 'admin' AND active) FROM users WHERE id = $1")
        .bind(target)
        .fetch_optional(pool)
        .await?;

    Ok(is_admin.unwrap_or(false) && others == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_long_random_and_recognisable() {
        let token = generate_token();
        assert!(token.starts_with("kasl_"), "a token in a log should be identifiable: {token}");
        assert_eq!(token.len(), 5 + 64, "32 bytes as hex");
        assert_ne!(token, generate_token(), "two tokens must never be the same");
    }

    #[test]
    fn a_short_password_is_refused_before_it_is_hashed() {
        let error = hash_new_password("short").expect_err("seven characters is not a password");
        assert!(error.to_string().contains("at least 8"), "{error}");
        assert!(hash_new_password("just long enough").is_ok());
    }

    #[test]
    fn the_email_check_admits_addresses_and_refuses_obvious_mistakes() {
        for good in ["a@b.co", "first.last@example.com", "kirill+kasl@example.co.uk"] {
            assert!(looks_like_an_email(good), "{good} should be accepted");
        }
        // What an admin actually mistypes: a name, a missing domain, a stray
        // trailing dot from a copied sentence.
        for bad in ["kirill", "kirill@", "@example.com", "kirill@example", "kirill@example.com."] {
            assert!(!looks_like_an_email(bad), "{bad} should be refused");
        }
    }

    #[test]
    fn a_manager_reads_and_an_employee_does_not() {
        let user = |role| CurrentUser {
            session_id: Uuid::nil(),
            user_id: Uuid::nil(),
            role,
            email: "someone@example.test".to_string(),
        };
        assert!(require_manager_or_admin(&user(UserRole::Admin)).is_ok());
        assert!(require_manager_or_admin(&user(UserRole::Manager)).is_ok());
        assert!(require_manager_or_admin(&user(UserRole::Employee)).is_err());

        // And reading is all a manager gets in this version.
        assert!(user(UserRole::Manager).require_admin().is_err());
    }

    #[test]
    fn an_absent_patch_field_means_leave_it_alone() {
        // `coalesce($n, column)` in the UPDATE relies on this: a field the admin
        // did not send must arrive as None, not as a default that clears it.
        let patch: UserPatch = serde_json::from_value(serde_json::json!({"display_name": "Kirill"})).unwrap();
        assert_eq!(patch.display_name.as_deref(), Some("Kirill"));
        assert!(patch.role.is_none() && patch.active.is_none() && patch.password.is_none());
    }

    #[test]
    fn a_new_user_defaults_to_the_least_authority() {
        let new: NewUser = serde_json::from_value(serde_json::json!({"email": "a@b.co"})).unwrap();
        assert_eq!(new.role, UserRole::Employee, "a role must be asked for, never assumed");
        assert!(new.password.is_none(), "an account for an agent needs no password");
    }
}
