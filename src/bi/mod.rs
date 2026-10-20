//! The BI API: `GET /api/bi/v1/...` (ADR 0025).
//!
//! What a dashboard tool reads the team's hours from - Grafana through its
//! Infinity data source, Power Query, a script on a timer - with a read token
//! rather than a session. Three rules, settled before the code:
//!
//! * **A contract of its own.** `/api/v1` is the web UI's API and grows with
//!   the screens. A dashboard somebody built in Grafana breaks silently, with
//!   nobody there to notice, so this one is versioned apart: within `v1` a
//!   dataset only ever gains columns at its end, and a column renamed, removed
//!   or retyped is `v2`. The columns are listed once, below, and the test suite
//!   holds them to the published list.
//! * **Rows, not documents.** Every dataset is a flat table - the shape a BI
//!   tool reads without a transformation - and the same thing has the same
//!   column name everywhere, so `people` joins `days` on `user_id`. Each comes
//!   as JSON, an array of objects, and as CSV: the same columns, the same
//!   values, durations in whole seconds and moments in UTC.
//! * **The server's rules, not a second copy of them.** The days are the
//!   timesheet's cells over any range (ADR 0024) and the summary is the team
//!   table's rows (ADR 0023). Who is in them is who the token's owner may see,
//!   by the one clause that decides it everywhere (ADR 0009).

pub mod table;
pub mod token;

use axum::{
    extract::{Query, State},
    response::Response,
};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    admin::{Reader, VISIBLE_USERS},
    app::AppState,
    error::ApiError,
    me::{self, Range},
    model::UserRole,
    team,
    timesheet::Sheet,
};
use table::{Table, Value};
use token::{Hours, ReadToken};

// The columns: the contract --------------------------------------------------------
//
// Append only. A column renamed, removed or given a different meaning is a new
// version of this API, not a change to this one.

/// One row per person the token's owner may see, departed ones included: a
/// day's `user_id` always finds its person.
pub const PEOPLE: &[&str; 9] = &[
    "user_id",
    "display_name",
    "email",
    "role",
    "active",
    "department_id",
    "department",
    "work_rate",
    "created_at",
];

/// One row per department.
pub const DEPARTMENTS: &[&str; 4] = &["department_id", "department", "manager_id", "manager"];

/// One row per person and date of the range - every date, recorded or not,
/// the way the timesheet answers it.
pub const DAYS: &[&str; 17] = &[
    "date",
    "user_id",
    "display_name",
    "email",
    "department_id",
    "department",
    "state",
    "kind",
    "started_at",
    "ended_at",
    "worked_seconds",
    "paused_seconds",
    "norm_seconds",
    "filed",
    "report",
    "calendar",
    "weekend",
];

/// One row per person over the range: the team table's row.
pub const SUMMARY: &[&str; 14] = &[
    "user_id",
    "display_name",
    "email",
    "department_id",
    "department",
    "active",
    "work_rate",
    "days_worked",
    "days_away",
    "worked_seconds",
    "paused_seconds",
    "norm_seconds",
    "due_seconds",
    "last_day",
];

// The datasets -------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct Person {
    id: Uuid,
    display_name: String,
    email: String,
    role: UserRole,
    active: bool,
    department_id: Option<Uuid>,
    department: Option<String>,
    work_rate: Decimal,
    created_at: DateTime<Utc>,
}

async fn people(pool: &PgPool, reader: Reader) -> Result<Table<9>, ApiError> {
    // `AssertSqlSafe` because the only interpolation is `VISIBLE_USERS`, a
    // constant in `admin`; the reader is bound.
    let people: Vec<Person> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT u.id, u.display_name, u.email, u.role, u.active, u.department_id, d.name AS department, u.work_rate, u.created_at
         FROM users u
         LEFT JOIN departments d ON d.id = u.department_id
         WHERE {VISIBLE_USERS}
         ORDER BY u.display_name, u.email"
    )))
    .bind(reader.is_admin())
    .bind(reader.user_id)
    .fetch_all(pool)
    .await?;

    let mut table = Table::new(PEOPLE);
    table.rows = people
        .into_iter()
        .map(|person| {
            [
                Value::Id(person.id),
                Value::Text(person.display_name),
                Value::Text(person.email),
                Value::word(person.role),
                Value::Flag(person.active),
                Value::or_null(person.department_id, Value::Id),
                Value::or_null(person.department, Value::Text),
                Value::Share(person.work_rate),
                Value::Instant(person.created_at),
            ]
        })
        .collect();
    Ok(table)
}

#[derive(sqlx::FromRow)]
struct Department {
    id: Uuid,
    name: String,
    manager_id: Option<Uuid>,
    manager: Option<String>,
}

async fn departments(pool: &PgPool) -> Result<Table<4>, ApiError> {
    // Every department, to anybody who may read the team: that a company has
    // a "Sales" is not the disclosure - the people in it are, and `people`
    // scopes them (ADR 0009).
    let departments: Vec<Department> = sqlx::query_as(
        "SELECT d.id, d.name, d.manager_id, m.display_name AS manager
         FROM departments d
         LEFT JOIN users m ON m.id = d.manager_id
         ORDER BY d.name",
    )
    .fetch_all(pool)
    .await?;

    let mut table = Table::new(DEPARTMENTS);
    table.rows = departments
        .into_iter()
        .map(|department| {
            [
                Value::Id(department.id),
                Value::Text(department.name),
                Value::or_null(department.manager_id, Value::Id),
                Value::or_null(department.manager, Value::Text),
            ]
        })
        .collect();
    Ok(table)
}

async fn days(pool: &PgPool, reader: Reader, range: &Range) -> Result<Table<17>, ApiError> {
    me::validate_range(range)?;
    let sheet = Sheet::load(pool, reader, range, None).await?;

    let mut table = Table::new(DAYS);
    table.rows.reserve(sheet.members.len() * sheet.dates.len());
    for member in &sheet.members {
        for column in &sheet.dates {
            let cell = sheet.cell(member, column.date);
            let day = sheet.recorded(member, column.date);
            table.rows.push([
                Value::Date(column.date),
                Value::Id(member.id),
                Value::Text(member.display_name.clone()),
                Value::Text(member.email.clone()),
                Value::or_null(member.department_id, Value::Id),
                Value::or_null(member.department.clone(), Value::Text),
                Value::word(cell.state),
                Value::or_null(cell.kind, Value::word),
                Value::or_null(day.map(|day| day.started_at), Value::Instant),
                Value::or_null(day.and_then(|day| day.ended_at), Value::Instant),
                Value::or_null(cell.worked_seconds, Value::Int),
                Value::or_null(day.map(|day| day.paused_seconds), Value::Int),
                Value::Int(cell.norm_seconds),
                Value::or_null(cell.filed, Value::word),
                Value::or_null(cell.report, Value::word),
                Value::or_null(column.calendar, Value::word),
                Value::Flag(column.weekend),
            ]);
        }
    }
    Ok(table)
}

async fn summary(pool: &PgPool, reader: Reader, range: &Range) -> Result<Table<14>, ApiError> {
    me::validate_range(range)?;
    let (members, _) = team::members(pool, reader, range, None).await?;

    let mut table = Table::new(SUMMARY);
    table.rows = members
        .into_iter()
        .map(|member| {
            [
                Value::Id(member.id),
                Value::Text(member.display_name),
                Value::Text(member.email),
                Value::or_null(member.department_id, Value::Id),
                Value::or_null(member.department, Value::Text),
                Value::Flag(member.active),
                Value::Share(member.work_rate),
                Value::Int(member.days_recorded),
                Value::Int(member.days_away),
                Value::Int(member.worked_seconds),
                Value::Int(member.paused_seconds),
                Value::Int(member.norm_seconds),
                Value::Int(member.due_seconds),
                Value::or_null(member.last_day, Value::Date),
            ]
        })
        .collect();
    Ok(table)
}

// The routes ---------------------------------------------------------------------

/// `GET /api/bi/v1/people`
pub async fn people_json(State(state): State<AppState>, token: ReadToken<Hours>) -> Result<Response, ApiError> {
    Ok(people(&state.pool, token.reader).await?.json())
}

/// `GET /api/bi/v1/people.csv`
pub async fn people_csv(State(state): State<AppState>, token: ReadToken<Hours>) -> Result<Response, ApiError> {
    Ok(people(&state.pool, token.reader).await?.csv())
}

/// `GET /api/bi/v1/departments`
pub async fn departments_json(State(state): State<AppState>, _token: ReadToken<Hours>) -> Result<Response, ApiError> {
    Ok(departments(&state.pool).await?.json())
}

/// `GET /api/bi/v1/departments.csv`
pub async fn departments_csv(State(state): State<AppState>, _token: ReadToken<Hours>) -> Result<Response, ApiError> {
    Ok(departments(&state.pool).await?.csv())
}

/// `GET /api/bi/v1/days?from=YYYY-MM-DD&to=YYYY-MM-DD`
pub async fn days_json(State(state): State<AppState>, token: ReadToken<Hours>, Query(range): Query<Range>) -> Result<Response, ApiError> {
    Ok(days(&state.pool, token.reader, &range).await?.json())
}

/// `GET /api/bi/v1/days.csv?from=YYYY-MM-DD&to=YYYY-MM-DD`
pub async fn days_csv(State(state): State<AppState>, token: ReadToken<Hours>, Query(range): Query<Range>) -> Result<Response, ApiError> {
    Ok(days(&state.pool, token.reader, &range).await?.csv())
}

/// `GET /api/bi/v1/summary?from=YYYY-MM-DD&to=YYYY-MM-DD`
pub async fn summary_json(State(state): State<AppState>, token: ReadToken<Hours>, Query(range): Query<Range>) -> Result<Response, ApiError> {
    Ok(summary(&state.pool, token.reader, &range).await?.json())
}

/// `GET /api/bi/v1/summary.csv?from=YYYY-MM-DD&to=YYYY-MM-DD`
pub async fn summary_csv(State(state): State<AppState>, token: ReadToken<Hours>, Query(range): Query<Range>) -> Result<Response, ApiError> {
    Ok(summary(&state.pool, token.reader, &range).await?.csv())
}
