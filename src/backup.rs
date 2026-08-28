//! `kasl-server backup` and `restore`: the installation's data as one file.
//!
//! An operator running this on their own hardware owns the consequences of
//! losing it, so taking a copy has to be something they can do without knowing
//! PostgreSQL. `pg_dump` is the better tool for someone who already has a
//! backup regime; this is for everyone else, and it does two things `pg_dump`
//! cannot:
//!
//! * **It knows the schema version.** A dump carries the migration it was
//!   taken at, and a restore refuses a file from a newer server rather than
//!   loading rows into tables whose columns have since changed. The failure
//!   mode being avoided is not an error - it is a restore that appears to work
//!   and quietly drops what the older schema has no place for.
//! * **It is the same binary.** No `postgres-client` package on the host, no
//!   version skew between a client and the server it dumps.
//!
//! JSON Lines rather than SQL: each line is a table's worth of rows, so a file
//! can be read by anything, checked by eye, and streamed without holding the
//! installation in memory.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row, postgres::PgRow};
use std::io::{BufRead, Write};

/// Tables in the order they must be written and loaded.
///
/// Parents first: every foreign key points at a table earlier in this list, so
/// a restore that walks it in order never inserts a row whose owner is missing.
/// `_sqlx_migrations` is deliberately absent - the schema is applied by the
/// server, not carried in the file.
///
/// `users` and `departments` are the exception no ordering can solve: a person
/// belongs to a department and a department names its manager, so each points
/// at the other. See [`DEFERRED_COLUMNS`].
const TABLES: [&str; 11] = [
    "users",
    "departments",
    "agents",
    "sessions",
    "workdays",
    "pauses",
    "tasks",
    "tags",
    "task_tags",
    "reports",
    "audit_log",
];

/// Columns held back on insert and written once every table is loaded.
///
/// The one cycle in this schema: `users.department_id` points at a department,
/// and `departments.manager_id` points back at a user. Whichever goes in first
/// has a column referring to a row that does not exist yet, so that column
/// waits - the row is inserted without it and updated at the end.
///
/// The alternative, making the constraints `DEFERRABLE`, would loosen them for
/// every ordinary write in the product to serve one command that runs rarely.
const DEFERRED_COLUMNS: [(&str, &str); 2] = [("users", "department_id"), ("departments", "manager_id")];

/// Tables the restore replaces wholesale but never empties from the file.
///
/// `settings` is one row created by a migration. Deleting it and inserting the
/// backup's copy would work, but leaving the row alone and updating it keeps
/// the `CHECK (singleton)` constraint honest at every moment.
const SETTINGS: &str = "settings";

/// What the file says about itself, on its first line.
#[derive(Debug, Serialize, Deserialize)]
pub struct Header {
    /// Always `kasl-server-backup`, so a file fed to the wrong tool says so.
    pub format: String,
    /// The migration the source database was at. The restore's gate.
    pub schema_version: i64,
    /// The version that wrote it - for a human reading the file, not a check.
    pub server_version: String,
    pub taken_at: chrono::DateTime<chrono::Utc>,
}

/// The marker every backup starts with.
const FORMAT: &str = "kasl-server-backup";

/// One table's rows.
#[derive(Debug, Serialize, Deserialize)]
struct Chunk {
    table: String,
    rows: Vec<serde_json::Value>,
}

/// Writes the whole installation to `out`.
///
/// Rows are read a table at a time and written as they come, so the file is
/// produced without the installation ever being resident in memory.
pub async fn dump(pool: &PgPool, schema_version: i64, out: &mut impl Write) -> Result<Summary> {
    let header = Header {
        format: FORMAT.to_string(),
        schema_version,
        server_version: env!("CARGO_PKG_VERSION").to_string(),
        taken_at: chrono::Utc::now(),
    };
    writeln!(out, "{}", serde_json::to_string(&header)?)?;

    let mut summary = Summary::default();

    for table in TABLES.into_iter().chain([SETTINGS]) {
        // `row_to_json` hands the whole row over as JSON, so this module does
        // not need a struct per table - and a column added by a later
        // migration travels without anything here being edited.
        // `AssertSqlSafe` on a name from `TABLES`, a constant in this file.
        let rows: Vec<serde_json::Value> = sqlx::query(sqlx::AssertSqlSafe(format!("SELECT row_to_json(t) AS row FROM {table} AS t")))
            .fetch_all(pool)
            .await
            .with_context(|| format!("failed to read {table}"))?
            .into_iter()
            .map(|row: PgRow| row.get::<serde_json::Value, _>("row"))
            .collect();

        summary.rows += rows.len();
        summary.tables += 1;
        writeln!(
            out,
            "{}",
            serde_json::to_string(&Chunk {
                table: table.to_string(),
                rows,
            })?
        )?;
    }

    out.flush()?;
    Ok(summary)
}

/// What a backup or restore moved.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub tables: usize,
    pub rows: usize,
}

/// Reads a backup into an empty installation.
///
/// Refuses rather than merges: a restore into a database with data in it would
/// have to decide what wins, and every answer to that is wrong for somebody.
/// The operator empties the database - or points at a fresh one - and the
/// intent is theirs rather than inferred.
pub async fn load(pool: &PgPool, schema_version: i64, input: impl BufRead) -> Result<Summary> {
    let mut lines = input.lines();

    let header: Header = {
        let first = lines.next().context("the backup is empty")??;
        serde_json::from_str(&first).context("the first line is not a kasl-server backup header")?
    };

    if header.format != FORMAT {
        bail!("this is not a kasl-server backup (format: {})", header.format);
    }

    // The gate this module exists for. A dump from a newer server may contain
    // columns this schema has no home for; loading it would drop them silently
    // and look like a success.
    if header.schema_version > schema_version {
        bail!(
            "the backup is from a newer server (schema {} against this server's {}); upgrade kasl-server before restoring",
            header.schema_version,
            schema_version
        );
    }

    ensure_empty(pool).await?;

    let mut summary = Summary::default();
    // One transaction: a restore that stopped halfway would leave an
    // installation whose rows reference owners that were never loaded.
    let mut tx = pool.begin().await?;

    // What the cycle forced us to leave out, to be written once every row it
    // could point at exists: (table, column, row id, value).
    let mut deferred: Vec<(&'static str, &'static str, serde_json::Value, serde_json::Value)> = Vec::new();

    for line in lines {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let chunk: Chunk = serde_json::from_str(&line).context("a line of the backup could not be read")?;

        summary.tables += 1;
        for row in &chunk.rows {
            let held = insert(&mut tx, &chunk.table, row).await?;
            deferred.extend(held);
            summary.rows += 1;
        }
    }

    for (table, column, id, value) in deferred {
        // Both names come from `DEFERRED_COLUMNS`, constants in this file.
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE {table} SET {column} = ($1::text)::uuid WHERE id = ($2::text)::uuid"
        )))
        .bind(value.as_str().unwrap_or_default())
        .bind(id.as_str().unwrap_or_default())
        .execute(&mut *tx)
        .await
        .with_context(|| format!("failed to restore {table}.{column}"))?;
    }

    tx.commit().await?;
    Ok(summary)
}

/// The one table name in this module that does not come from its own source.
///
/// A restore reads names out of a file, and a file can say anything. Matching
/// against the known list turns that name back into a constant before it
/// reaches a statement - the check is that the name *is* one of ours, not that
/// it looks harmless.
fn known_table(name: &str) -> Result<&'static str> {
    TABLES
        .into_iter()
        .chain([SETTINGS])
        .find(|table| *table == name)
        .with_context(|| format!("the backup names a table this server does not have: {name}"))
}

/// Inserts one row given as a JSON object.
///
/// `json_populate_record` turns the object back into a row of the table's own
/// type, so the column list lives in the database rather than being repeated
/// here for every table.
/// One value held back until the rest of the restore has caught up.
type Deferred = (&'static str, &'static str, serde_json::Value, serde_json::Value);

async fn insert(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, table: &str, row: &serde_json::Value) -> Result<Vec<Deferred>> {
    let table = known_table(table)?;

    // Take out the column that points into the cycle, if this table has one
    // and this row actually uses it. A null needs nothing held back.
    let mut held = Vec::new();
    let mut row = row.clone();
    for (owner, column) in DEFERRED_COLUMNS {
        if owner != table {
            continue;
        }
        let Some(object) = row.as_object_mut() else { continue };
        match object.get(column) {
            Some(serde_json::Value::Null) | None => {}
            Some(value) => {
                let value = value.clone();
                let id = object.get("id").cloned().unwrap_or(serde_json::Value::Null);
                object.insert(column.to_string(), serde_json::Value::Null);
                held.push((owner, column, id, value));
            }
        }
    }
    let row = &row;

    if table == SETTINGS {
        // The singleton row already exists, put there by the migration.
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE {SETTINGS} SET privacy_level = (r).privacy_level, updated_at = (r).updated_at
             FROM (SELECT json_populate_record(NULL::{SETTINGS}, $1::json) AS r) AS s
             WHERE singleton"
        )))
        .bind(row)
        .execute(&mut **tx)
        .await
        .with_context(|| format!("failed to restore {table}"))?;
        return Ok(held);
    }

    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO {table} SELECT * FROM json_populate_record(NULL::{table}, $1::json)"
    )))
    .bind(row)
    .execute(&mut **tx)
    .await
    .with_context(|| format!("failed to restore a row of {table}"))?;
    Ok(held)
}

/// Refuses to restore over an installation that already holds people.
///
/// `users` alone: everything else hangs off it, and an operator who has
/// started a server but never signed anyone in should not have to empty a
/// database to restore into it.
async fn ensure_empty(pool: &PgPool) -> Result<()> {
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users").fetch_one(pool).await?;
    if users > 0 {
        bail!("this database already holds {users} accounts; restore into an empty one");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_backup_names_itself_and_its_schema() {
        // The header is the whole contract with a future restore: a file that
        // did not say which schema it came from could only be loaded on faith.
        let header = Header {
            format: FORMAT.to_string(),
            schema_version: 20260826000001,
            server_version: "0.14.0".to_string(),
            taken_at: chrono::Utc::now(),
        };
        let json = serde_json::to_string(&header).unwrap();
        let read: Header = serde_json::from_str(&json).unwrap();

        assert_eq!(read.format, FORMAT);
        assert_eq!(read.schema_version, 20260826000001);
    }

    #[test]
    fn parents_come_before_their_children() {
        // The restore inserts in this order, so a table must never appear
        // before one it points at. Checked as a property rather than by eye:
        // a table added in the wrong place fails here instead of failing a
        // customer's restore with a foreign key violation.
        let position = |table: &str| TABLES.iter().position(|t| *t == table).unwrap_or_else(|| panic!("{table} is not in TABLES"));

        for (child, parent) in [
            ("agents", "users"),
            ("sessions", "users"),
            ("workdays", "users"),
            ("pauses", "workdays"),
            ("tasks", "users"),
            ("tags", "users"),
            ("task_tags", "tasks"),
            ("task_tags", "tags"),
            ("reports", "users"),
            ("departments", "users"),
        ] {
            assert!(position(parent) < position(child), "{parent} must be restored before {child}");
        }
    }

    #[test]
    fn a_table_name_from_a_file_has_to_be_one_of_ours() {
        // The only name in this module that does not come from its own source
        // arrives inside a backup, and a file can say anything. Every table
        // reaches a statement only after being matched back to a constant.
        assert_eq!(known_table("users").unwrap(), "users");
        assert_eq!(known_table(SETTINGS).unwrap(), SETTINGS);

        for hostile in ["users; DROP TABLE users", "pg_shadow", "_sqlx_migrations", "users --", ""] {
            let error = known_table(hostile).unwrap_err();
            assert!(error.to_string().contains("does not have"), "`{hostile}` must be refused by name, got: {error}");
        }
    }

    #[test]
    fn every_table_is_listed_once() {
        let mut seen = TABLES.to_vec();
        seen.sort_unstable();
        let count = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), count, "a table listed twice would be restored twice");
        assert!(!TABLES.contains(&SETTINGS), "settings is updated, not inserted, so it must not be in TABLES");
        assert!(
            !TABLES.contains(&"_sqlx_migrations"),
            "the schema is applied by the server, not carried in a backup"
        );
    }
}
