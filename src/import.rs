//! Importing an agent's local history.
//!
//! A team that adopts kasl-server late has employees who tracked their time
//! locally for months. That history is in an ordinary SQLite file on their
//! machine, and it should not be thrown away because the server arrived after
//! it. This reads that file and writes the days into the server's tables.
//!
//! The awkward part is time. kasl stores bare wall-clock text - `datetime(...,
//! 'localtime')`, no offset anywhere - which is unambiguous on the one machine
//! that wrote it and meaningless to a server serving several time zones. There
//! is nothing in the file to recover the offset from, so whoever runs the
//! import states it, and the choice is theirs to make and to get wrong (ADR
//! 0003, ADR 0006).

use anyhow::{Context, Result, bail};
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, TimeZone};
use rusqlite::Connection;
use sqlx::PgPool;
use uuid::Uuid;

/// The format kasl's `datetime()` writes: no offset, no fractional seconds.
const AGENT_TIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

/// One workday as it appears in the agent's database.
#[derive(Debug)]
pub struct AgentDay {
    pub date: NaiveDate,
    pub start: NaiveDateTime,
    pub end: Option<NaiveDateTime>,
    pub pauses: Vec<AgentPause>,
    pub tasks: Vec<AgentTask>,
}

#[derive(Debug)]
pub struct AgentPause {
    pub start: NaiveDateTime,
    pub end: Option<NaiveDateTime>,
    pub duration_seconds: Option<i32>,
    /// True for a break the employee entered by hand. In the agent these live
    /// in a separate `breaks` table; here they are pauses with a flag, which is
    /// the shape the server's schema already had.
    pub manual: bool,
    pub reason: Option<String>,
}

#[derive(Debug)]
pub struct AgentTask {
    pub agent_task_id: i32,
    pub agent_group_id: i32,
    pub recorded_at: NaiveDateTime,
    pub name: String,
    pub comment: Option<String>,
    pub completeness: i16,
}

/// What an import did, for the operator to read back.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportSummary {
    pub days: usize,
    pub pauses: usize,
    pub tasks: usize,
    /// Tasks the employee had deleted. Counted rather than silently dropped:
    /// "892 tasks" and "892 tasks, 14 skipped" describe different files.
    pub skipped_deleted_tasks: usize,
    /// Rows whose timestamps could not be parsed at all.
    pub skipped_unreadable: usize,
}

/// Reads an agent's database into days, newest last.
///
/// Opened read-only: this is the employee's file, and an import must not be
/// able to damage the thing it is copying from - not even by taking a write
/// lock on a database the agent is still using.
pub fn read_agent_db(path: &std::path::Path) -> Result<(Vec<AgentDay>, ImportSummary)> {
    if !path.exists() {
        bail!("no such file: {}", path.display());
    }

    let connection = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("failed to open {} as a SQLite database", path.display()))?;

    let mut summary = ImportSummary::default();
    let mut days = read_workdays(&connection, &mut summary)?;

    let pauses = read_pauses(&connection, &mut summary)?;
    let breaks = read_breaks(&connection, &mut summary)?;
    let tasks = read_tasks(&connection, &mut summary)?;

    // The agent has no foreign keys: it relates rows by comparing date strings,
    // and so must this. A pause whose date matches no workday is dropped rather
    // than inventing a day the employee never had.
    for (date, pause) in pauses.into_iter().chain(breaks) {
        if let Some(day) = days.iter_mut().find(|day| day.date == date) {
            day.pauses.push(pause);
            summary.pauses += 1;
        }
    }
    for (date, task) in tasks {
        if let Some(day) = days.iter_mut().find(|day| day.date == date) {
            day.tasks.push(task);
            summary.tasks += 1;
        }
    }

    summary.days = days.len();
    Ok((days, summary))
}

fn read_workdays(connection: &Connection, summary: &mut ImportSummary) -> Result<Vec<AgentDay>> {
    let mut statement = connection
        .prepare("SELECT date, start, end FROM workdays ORDER BY date")
        .context("failed to read the workdays table; is this a kasl database?")?;

    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?))
    })?;

    let mut days = Vec::new();
    for row in rows {
        let (date, start, end) = row?;
        let (Ok(date), Ok(start)) = (parse_date(&date), parse_time(&start)) else {
            summary.skipped_unreadable += 1;
            continue;
        };
        days.push(AgentDay {
            date,
            start,
            end: end.as_deref().and_then(|end| parse_time(end).ok()),
            pauses: Vec::new(),
            tasks: Vec::new(),
        });
    }

    Ok(days)
}

fn read_pauses(connection: &Connection, summary: &mut ImportSummary) -> Result<Vec<(NaiveDate, AgentPause)>> {
    let mut statement = connection.prepare("SELECT start, end, duration FROM pauses ORDER BY start")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<i32>>(2)?))
    })?;

    let mut pauses = Vec::new();
    for row in rows {
        let (start, end, duration_seconds) = row?;
        let Ok(start) = parse_time(&start) else {
            summary.skipped_unreadable += 1;
            continue;
        };
        pauses.push((
            start.date(),
            AgentPause {
                start,
                end: end.as_deref().and_then(|end| parse_time(end).ok()),
                duration_seconds,
                manual: false,
                reason: None,
            },
        ));
    }

    Ok(pauses)
}

/// Manual breaks, which the agent keeps in their own table.
///
/// Missing in databases from before the agent's migration 6, so a failure to
/// read it is not a failure to import: an older file simply has no breaks.
fn read_breaks(connection: &Connection, summary: &mut ImportSummary) -> Result<Vec<(NaiveDate, AgentPause)>> {
    let Ok(mut statement) = connection.prepare("SELECT date, start_time, end_time, duration, reason FROM breaks ORDER BY start_time") else {
        return Ok(Vec::new());
    };

    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<i32>>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;

    let mut breaks = Vec::new();
    for row in rows {
        let (date, start, end, duration_seconds, reason) = row?;
        let (Ok(date), Ok(start)) = (parse_date(&date), parse_time(&start)) else {
            summary.skipped_unreadable += 1;
            continue;
        };
        breaks.push((
            date,
            AgentPause {
                start,
                end: end.as_deref().and_then(|end| parse_time(end).ok()),
                duration_seconds,
                manual: true,
                reason,
            },
        ));
    }

    Ok(breaks)
}

fn read_tasks(connection: &Connection, summary: &mut ImportSummary) -> Result<Vec<(NaiveDate, AgentTask)>> {
    // `deleted_at` arrived in the agent's migration 4; older files lack the
    // column, so the query that filters on it is tried first and the plain one
    // is the fallback.
    let (sql, filters_deleted) =
        match connection.prepare("SELECT id, task_id, timestamp, name, comment, completeness FROM tasks WHERE deleted_at IS NULL ORDER BY id") {
            Ok(_) => (
                "SELECT id, task_id, timestamp, name, comment, completeness FROM tasks WHERE deleted_at IS NULL ORDER BY id",
                true,
            ),
            Err(_) => ("SELECT id, task_id, timestamp, name, comment, completeness FROM tasks ORDER BY id", false),
        };

    if filters_deleted {
        let deleted: i64 = connection
            .query_row("SELECT count(*) FROM tasks WHERE deleted_at IS NOT NULL", [], |row| row.get(0))
            .unwrap_or(0);
        summary.skipped_deleted_tasks = deleted.max(0) as usize;
    }

    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i32>(0)?,
            row.get::<_, i32>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, i32>(5)?,
        ))
    })?;

    let mut tasks = Vec::new();
    for row in rows {
        let (agent_task_id, task_id, recorded_at, name, comment, completeness) = row?;
        let Ok(recorded_at) = parse_time(&recorded_at) else {
            summary.skipped_unreadable += 1;
            continue;
        };
        tasks.push((
            recorded_at.date(),
            AgentTask {
                agent_task_id,
                // The agent stores 0 for "belongs to itself"; the server stores
                // the task's own id, as a live upload would send.
                agent_group_id: if task_id == 0 { agent_task_id } else { task_id },
                recorded_at,
                name,
                comment,
                // Clamped rather than refused: a value outside 0..=100 is a bug
                // in an old agent, not a reason to lose the employee's day.
                completeness: completeness.clamp(0, 100) as i16,
            },
        ));
    }

    Ok(tasks)
}

fn parse_date(raw: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d").with_context(|| format!("not a date: {raw}"))
}

fn parse_time(raw: &str) -> Result<NaiveDateTime> {
    let raw = raw.trim();
    NaiveDateTime::parse_from_str(raw, AGENT_TIME_FORMAT)
        // Some rows carry fractional seconds, depending on how they were
        // written; accept both rather than dropping the day over a decimal.
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f"))
        .with_context(|| format!("not a timestamp: {raw}"))
}

/// Keeps only the days within an inclusive date range.
///
/// The range exists for the employee who changed time zones mid-history: one
/// run per stretch, each with the offset that stretch was recorded in. Both
/// ends are optional, and an absent one means "no bound on that side".
pub fn within(days: Vec<AgentDay>, since: Option<NaiveDate>, until: Option<NaiveDate>) -> Vec<AgentDay> {
    days.into_iter()
        .filter(|day| since.is_none_or(|since| day.date >= since) && until.is_none_or(|until| day.date <= until))
        .collect()
}

/// Applies the operator's offset to a wall-clock time from the agent.
///
/// A fixed offset, not a zone: there is nothing in the file to say which of two
/// possible offsets a given day was recorded in, so one is chosen for all of
/// them and stated plainly in the output (ADR 0006).
pub fn at_offset(time: NaiveDateTime, offset: FixedOffset) -> DateTime<FixedOffset> {
    // `LocalResult` cannot be ambiguous for a fixed offset: it has no
    // transitions. The single mapping is always there.
    offset
        .from_local_datetime(&time)
        .single()
        .expect("a fixed offset maps every local time exactly once")
}

/// Writes the days into the server's tables, as the given user.
///
/// Each day is its own transaction, matching the batch upload: an import of a
/// year that fails on day 200 leaves 199 days imported, and running it again
/// is safe because a re-imported day replaces itself.
pub async fn write_days(pool: &PgPool, user_id: Uuid, days: &[AgentDay], offset: FixedOffset) -> Result<usize> {
    let mut written = 0;

    for day in days {
        let mut tx = pool.begin().await?;
        write_day(&mut tx, user_id, day, offset).await?;
        tx.commit().await?;
        written += 1;
    }

    Ok(written)
}

/// Writes one day inside a transaction the caller owns.
///
/// `write_days` commits each day on its own, which is right for an import
/// that must survive failing halfway. The demo has the opposite need - a
/// person's whole history in one commit, because four thousand commits is
/// the difference between a demo that opens in seconds and one that does not.
pub async fn write_day(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, user_id: Uuid, day: &AgentDay, offset: FixedOffset) -> Result<()> {
    let workday_id: Uuid = sqlx::query_scalar(
        "INSERT INTO workdays (user_id, date, started_at, ended_at) VALUES ($1, $2, $3, $4)
         ON CONFLICT (user_id, date) DO UPDATE SET started_at = EXCLUDED.started_at, ended_at = EXCLUDED.ended_at
         RETURNING id",
    )
    .bind(user_id)
    .bind(day.date)
    .bind(at_offset(day.start, offset))
    .bind(day.end.map(|end| at_offset(end, offset)))
    .fetch_one(&mut **tx)
    .await
    .with_context(|| format!("failed to write the workday of {}", day.date))?;

    sqlx::query("DELETE FROM pauses WHERE workday_id = $1")
        .bind(workday_id)
        .execute(&mut **tx)
        .await?;
    for pause in &day.pauses {
        sqlx::query("INSERT INTO pauses (workday_id, started_at, ended_at, duration_seconds, manual, reason) VALUES ($1, $2, $3, $4, $5, $6)")
            .bind(workday_id)
            .bind(at_offset(pause.start, offset))
            .bind(pause.end.map(|end| at_offset(end, offset)))
            .bind(pause.duration_seconds)
            .bind(pause.manual)
            .bind(pause.reason.as_deref())
            .execute(&mut **tx)
            .await?;
    }

    for task in &day.tasks {
        sqlx::query(
            "INSERT INTO tasks (user_id, agent_task_id, agent_group_id, date, recorded_at, name, comment, completeness)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (user_id, agent_task_id) DO UPDATE SET
                 agent_group_id = EXCLUDED.agent_group_id,
                 date = EXCLUDED.date,
                 recorded_at = EXCLUDED.recorded_at,
                 name = EXCLUDED.name,
                 comment = EXCLUDED.comment,
                 completeness = EXCLUDED.completeness",
        )
        .bind(user_id)
        .bind(task.agent_task_id)
        .bind(task.agent_group_id)
        .bind(day.date)
        .bind(at_offset(task.recorded_at, offset))
        .bind(task.name.trim())
        .bind(task.comment.as_deref())
        .bind(task.completeness)
        .execute(&mut **tx)
        .await?;
    }

    Ok(())
}

/// Finds the user an import writes for, refusing to invent one.
///
/// An import that created accounts would let a typo in an email address file a
/// year of someone's history under a person who does not exist, with nothing
/// to notice it. The account is made first, deliberately.
pub async fn resolve_user(pool: &PgPool, email: &str) -> Result<Uuid> {
    let user: Option<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE lower(email) = lower($1)")
        .bind(email)
        .fetch_optional(pool)
        .await?;

    user.with_context(|| format!("no user with the email {email}; create the account before importing into it"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_agents_timestamp_format() {
        let time = parse_time("2026-08-14 09:12:00").expect("the agent's own format must parse");
        assert_eq!(time.to_string(), "2026-08-14 09:12:00");

        // Written by some paths with a fraction; the day should not be lost.
        assert!(parse_time("2026-08-14 09:12:00.123").is_ok());
        // An offset is exactly what the agent never writes; if one appears, the
        // assumption behind this whole module is wrong and it should not parse.
        assert!(parse_time("2026-08-14T09:12:00-03:00").is_err());
    }

    #[test]
    fn the_operators_offset_makes_the_instant_absolute() {
        let time = parse_time("2026-08-14 09:12:00").unwrap();
        let offset = FixedOffset::east_opt(-3 * 3600).unwrap();

        let instant = at_offset(time, offset);
        assert_eq!(instant.to_rfc3339(), "2026-08-14T09:12:00-03:00");
        assert_eq!(instant.naive_utc().to_string(), "2026-08-14 12:12:00", "09:12-03:00 is 12:12 UTC");
    }

    #[test]
    fn a_different_offset_is_a_different_moment() {
        // The reason the argument is required rather than defaulted: the same
        // text becomes a different instant, and only the operator knows which.
        let time = parse_time("2026-08-14 09:12:00").unwrap();
        let west = at_offset(time, FixedOffset::east_opt(-3 * 3600).unwrap());
        let east = at_offset(time, FixedOffset::east_opt(5 * 3600).unwrap());

        assert_ne!(west.naive_utc(), east.naive_utc());
        assert_eq!(east.naive_utc().to_string(), "2026-08-14 04:12:00");
    }
}
