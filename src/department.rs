//! Departments: the boundary a manager's authority is measured in.
//!
//! Administered by an administrator, read by anyone who may read the team.
//! Membership is one department per person (`users.department_id`), and a
//! department names its own manager - so "who may see whom" is one join rather
//! than a rule to remember (ADR 0009).

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{app::AppState, error::ApiError, login::CurrentUser, model::UserRole};

/// A department as the admin screens list it.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct DepartmentRow {
    pub id: Uuid,
    pub name: String,
    pub manager_id: Option<Uuid>,
    /// The manager's display name, so a list needs no second request.
    pub manager: Option<String>,
    /// How many people are filed here. The number an admin actually looks at.
    pub members: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct NewDepartment {
    pub name: String,
    pub manager_id: Option<Uuid>,
}

/// A change to a department. Absent means "leave it alone"; `manager_id: null`
/// explicitly clears it, which is how a department between heads is recorded.
#[derive(Debug, Deserialize)]
pub struct DepartmentPatch {
    pub name: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub manager_id: Option<Option<Uuid>>,
}

/// Distinguishes an absent field from an explicit `null`.
///
/// Without this a request that omits `manager_id` and one that sets it to null
/// arrive identically, and the second - "this department has no head at the
/// moment" - becomes impossible to express.
fn double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    serde::Deserialize::deserialize(deserializer).map(Some)
}

#[derive(Debug, Deserialize)]
pub struct Assignment {
    /// Null removes the person from their department without deleting them.
    pub department_id: Option<Uuid>,
}

/// Lists departments. Readable by anyone who may read the team.
pub async fn list(State(state): State<AppState>, user: CurrentUser) -> Result<impl IntoResponse, ApiError> {
    if user.role == UserRole::Employee {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "not allowed"));
    }

    // Every department, both roles. A manager knowing the company has a "Sales"
    // is not a disclosure worth a second query path - the people inside it are
    // what `GET /users` scopes.
    let departments: Vec<DepartmentRow> = sqlx::query_as(
        "SELECT d.id, d.name, d.manager_id, m.display_name AS manager,
                (SELECT count(*) FROM users u WHERE u.department_id = d.id) AS members,
                d.created_at
         FROM departments d
         LEFT JOIN users m ON m.id = d.manager_id
         ORDER BY d.name",
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(departments))
}

/// Creates a department.
pub async fn create(State(state): State<AppState>, user: CurrentUser, Json(new): Json<NewDepartment>) -> Result<impl IntoResponse, ApiError> {
    user.require_admin()?;

    let name = new.name.trim();
    if name.is_empty() {
        return Err(ApiError::bad_request("a department needs a name"));
    }
    if let Some(manager_id) = new.manager_id {
        check_can_manage(&state.pool, manager_id).await?;
    }

    let created: Result<Uuid, sqlx::Error> = sqlx::query_scalar("INSERT INTO departments (name, manager_id) VALUES ($1, $2) RETURNING id")
        .bind(name)
        .bind(new.manager_id)
        .fetch_one(&state.pool)
        .await;

    let id = match created {
        Ok(id) => id,
        Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
            return Err(ApiError::new(StatusCode::CONFLICT, "a department with that name already exists"));
        }
        Err(error) => return Err(error.into()),
    };

    tracing::info!(%id, by = %user.user_id, "created a department");
    Ok((StatusCode::CREATED, Json(serde_json::json!({"id": id}))))
}

/// Renames a department or changes who runs it.
pub async fn update(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(target): Path<Uuid>,
    Json(patch): Json<DepartmentPatch>,
) -> Result<impl IntoResponse, ApiError> {
    user.require_admin()?;

    if let Some(Some(manager_id)) = patch.manager_id {
        check_can_manage(&state.pool, manager_id).await?;
    }

    let name = patch.name.as_deref().map(str::trim).filter(|name| !name.is_empty());
    // `manager_id` is three-valued here: absent leaves it, `Some(None)` clears
    // it, `Some(Some(id))` sets it. `coalesce` cannot express the middle one,
    // so the flag decides which branch the statement takes.
    let (set_manager, manager_id) = match patch.manager_id {
        None => (false, None),
        Some(value) => (true, value),
    };

    let updated = sqlx::query(
        "UPDATE departments SET
             name = coalesce($2, name),
             manager_id = CASE WHEN $3 THEN $4 ELSE manager_id END
         WHERE id = $1",
    )
    .bind(target)
    .bind(name)
    .bind(set_manager)
    .bind(manager_id)
    .execute(&state.pool)
    .await;

    let updated = match updated {
        Ok(result) => result.rows_affected(),
        Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
            return Err(ApiError::new(StatusCode::CONFLICT, "a department with that name already exists"));
        }
        Err(error) => return Err(error.into()),
    };

    if updated == 0 {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "no such department"));
    }

    tracing::info!(%target, by = %user.user_id, "updated a department");
    Ok(StatusCode::NO_CONTENT)
}

/// Removes a department. Its people stay, unfiled.
pub async fn delete(State(state): State<AppState>, user: CurrentUser, Path(target): Path<Uuid>) -> Result<impl IntoResponse, ApiError> {
    user.require_admin()?;

    // The foreign key is ON DELETE SET NULL, so the members survive and become
    // admin-only until they are filed again. Deleting people along with the
    // department they happened to be in would be a catastrophe behind a button.
    let deleted = sqlx::query("DELETE FROM departments WHERE id = $1")
        .bind(target)
        .execute(&state.pool)
        .await?
        .rows_affected();

    if deleted == 0 {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "no such department"));
    }

    tracing::info!(%target, by = %user.user_id, "deleted a department");
    Ok(StatusCode::NO_CONTENT)
}

/// Files a person into a department, or removes them from one.
pub async fn assign(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(target): Path<Uuid>,
    Json(assignment): Json<Assignment>,
) -> Result<impl IntoResponse, ApiError> {
    user.require_admin()?;

    let updated = sqlx::query("UPDATE users SET department_id = $2 WHERE id = $1")
        .bind(target)
        .bind(assignment.department_id);

    let updated = match updated.execute(&state.pool).await {
        Ok(result) => result.rows_affected(),
        // A department id that does not exist. Answered as a bad request
        // rather than a 500: the caller sent an id, and it was wrong.
        Err(sqlx::Error::Database(error)) if error.is_foreign_key_violation() => {
            return Err(ApiError::bad_request("no such department"));
        }
        Err(error) => return Err(error.into()),
    };

    if updated == 0 {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "no such user"));
    }

    tracing::info!(%target, department = ?assignment.department_id, by = %user.user_id, "assigned a department");
    Ok(StatusCode::NO_CONTENT)
}

/// Refuses to put someone in charge who cannot be in charge.
///
/// An employee heading a department would see nothing of it - `GET /users`
/// admits managers and admins only - so the department would silently have no
/// working head at all.
async fn check_can_manage(pool: &sqlx::PgPool, manager_id: Uuid) -> Result<(), ApiError> {
    let role: Option<(UserRole, bool)> = sqlx::query_as("SELECT role, active FROM users WHERE id = $1")
        .bind(manager_id)
        .fetch_optional(pool)
        .await?;

    match role {
        None => Err(ApiError::bad_request("no such user")),
        Some((_, false)) => Err(ApiError::bad_request("that account is deactivated")),
        Some((UserRole::Employee, _)) => Err(ApiError::bad_request("an employee cannot run a department; make them a manager first")),
        Some((UserRole::Manager | UserRole::Admin, true)) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_manager_and_an_explicit_null_are_different_requests() {
        // The whole reason for the double option: "leave the head alone" and
        // "this department has no head at the moment" must not be the same
        // payload.
        let absent: DepartmentPatch = serde_json::from_value(serde_json::json!({"name": "Sales"})).unwrap();
        assert!(absent.manager_id.is_none(), "an omitted field must leave the manager alone");

        let cleared: DepartmentPatch = serde_json::from_value(serde_json::json!({"manager_id": null})).unwrap();
        assert_eq!(cleared.manager_id, Some(None), "an explicit null must clear it");

        let set: DepartmentPatch = serde_json::from_value(serde_json::json!({"manager_id": "00000000-0000-0000-0000-000000000001"})).unwrap();
        assert!(matches!(set.manager_id, Some(Some(_))));
    }

    #[test]
    fn an_assignment_can_carry_nothing_which_means_unfiled() {
        let removed: Assignment = serde_json::from_value(serde_json::json!({"department_id": null})).unwrap();
        assert!(removed.department_id.is_none());
    }
}
