//! A fictional team, so the dashboards can be seen before a single agent is
//! installed on anybody's machine.
//!
//! Turned on with `KASL_DEMO=true`. On an empty database the server creates
//! three departments, their managers and employees, an agent for each, and
//! eight weeks of days shaped so every state the dashboard knows how to show
//! is on screen at once: someone steady, someone working long days, someone
//! whose hours are shrinking week by week, a day open right now, an agent gone
//! silent, and one that never reported at all.
//!
//! On a database that already holds accounts it refuses to start (ADR 0013).
//! The installation is marked as a demo in `settings`, which is what the web
//! UI reads to say "nothing here is real" - the environment variable can be
//! dropped later and the label stays.
//!
//! The team is generated, not stored: names, emails and the shape of every
//! day live in this file, and the generator is deterministic for a given
//! "today", so two demos started on the same date show the same numbers and a
//! screenshot can be reproduced.
//!
//! Every person's history is written through `import::write_days` - the path
//! an operator's own import takes - rather than through a private INSERT, so
//! the demo exercises the same rows the dashboards were built on.

use anyhow::{Context, Result, bail};
use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use chrono::{DateTime, Datelike, Duration, NaiveDateTime, NaiveTime, Utc, Weekday};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    app::AppState,
    audit,
    auth::hash_token,
    error::ApiError,
    heartbeat::{self, AgentState},
    import::{self, AgentDay, AgentPause, AgentTask},
    model::UserRole,
    session::hash_password,
};

/// The one password every demo account signs in with.
///
/// Fixed and documented, not generated: a demo server has nothing to protect,
/// and a visitor who has to dig three passwords out of a log before seeing a
/// dashboard has been handed a chore instead of a demo.
pub const PASSWORD: &str = "kasl-demo";

/// The domain every demo address is under. Reserved by RFC 2606, so no demo
/// account can ever be somebody's real one.
const DOMAIN: &str = "example.com";

/// How far back the history goes.
const WEEKS: i64 = 8;

/// How a person's days are shaped over the eight weeks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pattern {
    /// Roughly eight hours, a couple of interruptions, every weekday.
    Steady,
    /// Ten-hour days: the row the manager should be looking at.
    Long,
    /// A full week at the start, five-hour days by the end - the trend the
    /// "trends and anomalies" milestone will point at.
    Fading,
    /// Steady, but the breaks are entered by hand, with reasons.
    Breaks,
    /// Working right now: a day open on today's date.
    Open,
    /// Reported until a week ago, then nothing: the agent went quiet.
    Silent,
    /// Has an agent, has never sent a day.
    Never,
}

/// One member of the fictional team.
struct Person {
    first: &'static str,
    last: &'static str,
    role: UserRole,
    /// `None` for the administrator, who runs the installation rather than
    /// belonging to a department.
    department: Option<&'static str>,
    /// UTC offset in minutes. A distributed team, so the dashboard shows what
    /// it looks like when "09:00" means five different instants.
    offset_minutes: i32,
    /// `None` for someone who has no agent at all.
    pattern: Option<Pattern>,
}

impl Person {
    fn email(&self) -> String {
        format!("{}.{}@{DOMAIN}", self.first.to_ascii_lowercase(), self.last.to_ascii_lowercase())
    }

    fn display_name(&self) -> String {
        format!("{} {}", self.first, self.last)
    }

    /// The agent's bearer token. Fixed like the password, and for the same
    /// reason - and it means a real kasl can be pointed at the demo.
    fn token(&self) -> String {
        format!("demo-{}", self.first.to_ascii_lowercase())
    }

    fn offset(&self) -> chrono::FixedOffset {
        chrono::FixedOffset::east_opt(self.offset_minutes * 60).expect("the offsets in the team table are valid")
    }
}

/// A department and the email of the person who runs it.
struct Department {
    name: &'static str,
    manager: &'static str,
}

const DEPARTMENTS: [Department; 3] = [
    Department {
        name: "Engineering",
        manager: "priya.raman",
    },
    Department {
        name: "Design",
        manager: "daniel.okafor",
    },
    Department {
        name: "Support",
        manager: "elena.novak",
    },
];

/// The team. Order matters for the generator's seeds and for nothing else.
const TEAM: [Person; 12] = [
    Person {
        first: "Sam",
        last: "Whitfield",
        role: UserRole::Admin,
        department: None,
        offset_minutes: 0,
        pattern: None,
    },
    Person {
        first: "Priya",
        last: "Raman",
        role: UserRole::Manager,
        department: Some("Engineering"),
        offset_minutes: 5 * 60 + 30,
        pattern: Some(Pattern::Steady),
    },
    Person {
        first: "Daniel",
        last: "Okafor",
        role: UserRole::Manager,
        department: Some("Design"),
        offset_minutes: 60,
        pattern: Some(Pattern::Steady),
    },
    Person {
        first: "Elena",
        last: "Novak",
        role: UserRole::Manager,
        department: Some("Support"),
        offset_minutes: 2 * 60,
        pattern: Some(Pattern::Long),
    },
    Person {
        first: "Tomas",
        last: "Verhoeven",
        role: UserRole::Employee,
        department: Some("Engineering"),
        offset_minutes: 2 * 60,
        pattern: Some(Pattern::Steady),
    },
    Person {
        first: "Aiko",
        last: "Tanaka",
        role: UserRole::Employee,
        department: Some("Engineering"),
        offset_minutes: 9 * 60,
        pattern: Some(Pattern::Breaks),
    },
    Person {
        first: "Lukas",
        last: "Brandt",
        role: UserRole::Employee,
        department: Some("Engineering"),
        offset_minutes: 60,
        pattern: Some(Pattern::Fading),
    },
    Person {
        first: "Sofia",
        last: "Reyes",
        role: UserRole::Employee,
        department: Some("Engineering"),
        offset_minutes: -3 * 60,
        pattern: Some(Pattern::Open),
    },
    Person {
        first: "Mira",
        last: "Halvorsen",
        role: UserRole::Employee,
        department: Some("Design"),
        offset_minutes: 2 * 60,
        pattern: Some(Pattern::Steady),
    },
    Person {
        first: "Jonas",
        last: "Petit",
        role: UserRole::Employee,
        department: Some("Design"),
        offset_minutes: 60,
        pattern: Some(Pattern::Silent),
    },
    Person {
        first: "Yusuf",
        last: "Demir",
        role: UserRole::Employee,
        department: Some("Support"),
        offset_minutes: 3 * 60,
        pattern: Some(Pattern::Long),
    },
    Person {
        first: "Hana",
        last: "Kowalski",
        role: UserRole::Employee,
        department: Some("Support"),
        offset_minutes: 60,
        pattern: Some(Pattern::Never),
    },
];

/// What each department's people log their days against.
fn task_pool(department: &str) -> &'static [&'static str] {
    match department {
        "Engineering" => &[
            "Reliable ingest",
            "Batch upload retries",
            "Migration to the new query layer",
            "Review: departments API",
            "Flaky CI on arm64",
            "Onboarding checklist",
            "Release notes for 2.3",
            "Connection pool tuning",
        ],
        "Design" => &[
            "Dashboard empty states",
            "Icon set, second pass",
            "Login screen polish",
            "Design tokens audit",
            "Mobile layout study",
            "Timeline colours in dark mode",
        ],
        _ => &[
            "Ticket triage",
            "Customer call: Northbridge",
            "Knowledge base: backups",
            "Escalation: lost password",
            "Weekly support digest",
            "Renewal follow-ups",
        ],
    }
}

/// Reasons a person gives for a break they entered by hand.
const BREAK_REASONS: [&str; 5] = ["Lunch", "School run", "Dentist", "Walk", "Errand"];

/// An account a visitor can sign in as.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Account {
    pub role: UserRole,
    pub email: String,
    pub display_name: String,
}

/// One of each role, in the order a visitor would try them: the manager's
/// dashboard is what the demo exists to show, so it is not last.
pub fn showcase() -> Vec<Account> {
    [UserRole::Manager, UserRole::Employee, UserRole::Admin]
        .into_iter()
        .filter_map(|role| TEAM.iter().find(|person| person.role == role))
        .map(|person| Account {
            role: person.role,
            email: person.email(),
            display_name: person.display_name(),
        })
        .collect()
}

/// What the database says about itself before the demo touches it.
#[derive(Debug, PartialEq, Eq)]
pub enum Status {
    /// No accounts: the demo may seed.
    Empty,
    /// Already the demo: start and say nothing.
    Demo,
    /// Somebody's real installation. The demo must not run here.
    Populated { accounts: i64 },
}

pub async fn status(pool: &PgPool) -> Result<Status> {
    let demo: bool = sqlx::query_scalar("SELECT demo FROM settings WHERE singleton")
        .fetch_one(pool)
        .await
        .context("failed to read the installation's settings")?;
    if demo {
        return Ok(Status::Demo);
    }
    let accounts: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(pool)
        .await
        .context("failed to count accounts")?;
    Ok(if accounts == 0 { Status::Empty } else { Status::Populated { accounts } })
}

/// What a seed created.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Seeded {
    pub departments: usize,
    pub people: usize,
    pub days: usize,
}

/// Seeds the team into an empty database.
///
/// `now` is the moment the history is built back from: the last eight weeks
/// end yesterday, and the one open day is open as of this instant. Passed in
/// rather than read from the clock so a test can pin it.
///
/// Refuses a database that holds accounts already - the check is here as
/// well as in `status`, because this is the function that writes.
pub async fn seed(pool: &PgPool, now: DateTime<Utc>) -> Result<Seeded> {
    match status(pool).await? {
        Status::Empty => {}
        Status::Demo => bail!("this database already holds the demo team"),
        Status::Populated { accounts } => bail!("this database already holds {accounts} accounts; the demo only seeds an empty one"),
    }

    let mut seeded = Seeded::default();

    // People, departments and agents in one transaction, with the demo mark
    // first: if the process dies between here and the last day written, the
    // next start sees a demo and starts, rather than seeing accounts it did
    // not make and refusing.
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE settings SET demo = true WHERE singleton").execute(&mut *tx).await?;

    let mut department_ids = Vec::with_capacity(DEPARTMENTS.len());
    for department in &DEPARTMENTS {
        let id: Uuid = sqlx::query_scalar("INSERT INTO departments (name) VALUES ($1) RETURNING id")
            .bind(department.name)
            .fetch_one(&mut *tx)
            .await
            .with_context(|| format!("failed to create the {} department", department.name))?;
        department_ids.push((department.name, id));
        seeded.departments += 1;
    }
    let department_id = |name: &str| department_ids.iter().find(|(n, _)| *n == name).map(|(_, id)| *id);

    // One hash for one password: argon2 is deliberately slow, and twelve of
    // them at startup is a pause a visitor would notice.
    let password_hash = hash_password(PASSWORD)?;

    let mut user_ids = Vec::with_capacity(TEAM.len());
    for person in &TEAM {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO users (email, display_name, role, password_hash, active, department_id)
             VALUES ($1, $2, $3, $4, true, $5) RETURNING id",
        )
        .bind(person.email())
        .bind(person.display_name())
        .bind(person.role)
        .bind(&password_hash)
        .bind(person.department.and_then(department_id))
        .fetch_one(&mut *tx)
        .await
        .with_context(|| format!("failed to create the account for {}", person.display_name()))?;

        if person.pattern.is_some() {
            sqlx::query("INSERT INTO agents (user_id, name, token_hash) VALUES ($1, $2, $3)")
                .bind(id)
                .bind(format!("{}-laptop", person.first.to_ascii_lowercase()))
                .bind(hash_token(&person.token()))
                .execute(&mut *tx)
                .await
                .with_context(|| format!("failed to create the agent for {}", person.display_name()))?;
        }

        user_ids.push(id);
        seeded.people += 1;
    }

    for department in &DEPARTMENTS {
        let manager = TEAM
            .iter()
            .position(|person| person.email().starts_with(department.manager))
            .expect("every department in the table names a member of the team");
        sqlx::query("UPDATE departments SET manager_id = $1 WHERE name = $2")
            .bind(user_ids[manager])
            .bind(department.name)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;

    for (index, person) in TEAM.iter().enumerate() {
        let Some(pattern) = person.pattern else { continue };
        let days = days_for(person, pattern, index as u64, now);
        // One commit per person: the same rows an import writes, without the
        // per-day commit an import needs to survive failing halfway.
        let mut tx = pool.begin().await?;
        for day in &days {
            import::write_day(&mut tx, user_ids[index], day, person.offset()).await?;
        }
        tx.commit().await?;
        seeded.days += days.len();

        // When the server last heard from this machine: the end of the last
        // day for most, this very moment for the open one, never for the
        // agent that never reported. The dashboard's "last data 2 d ago" and
        // "never reported" come from here.
        let last_seen: Option<DateTime<Utc>> = match pattern {
            Pattern::Never => None,
            Pattern::Open => Some(now),
            _ => days
                .iter()
                .filter_map(|day| day.end)
                .max()
                .map(|end| import::at_offset(end, person.offset()).with_timezone(&Utc)),
        };
        let (pulse, age) = pulse_for(pattern);
        sqlx::query(
            "UPDATE agents SET last_seen_at = $2, heartbeat_state = $3, demo_pulse_age_seconds = $4,
                    heartbeat_at = CASE WHEN $3 IS NULL THEN NULL ELSE now() - coalesce($4, 0) * interval '1 second' END,
                    heartbeat_received_at = CASE WHEN $3 IS NULL THEN NULL ELSE now() - coalesce($4, 0) * interval '1 second' END
             WHERE user_id = $1",
        )
        .bind(user_ids[index])
        .bind(last_seen)
        .bind(pulse)
        .bind(age)
        .execute(pool)
        .await?;
    }

    audit::Entry::new(audit::action::DEMO_SEEDED)
        .with(serde_json::json!({ "people": seeded.people, "departments": seeded.departments, "days": seeded.days }))
        .record(pool)
        .await;

    Ok(seeded)
}

/// The pulse a pattern gets, and how old it is kept.
///
/// Only the people whose day is happening now get a live one; everyone else
/// has stopped for the day, which on a real installation is silence, and
/// silence is what a visitor should see it look like (ADR 0014).
fn pulse_for(pattern: Pattern) -> (Option<AgentState>, Option<i32>) {
    let state = match pattern {
        // Mid-day, at the keyboard - the row that reads "working".
        Pattern::Open => Some(AgentState::Working),
        // Also mid-day, but away from it: the demo needs both live states on
        // screen, or "paused" would never be seen.
        Pattern::Breaks => Some(AgentState::Paused),
        // Their agent is up and reporting, they are simply done for the day.
        // This is what distinguishes `idle` from `offline` - and the reason
        // the dashboard needs both.
        Pattern::Steady => Some(AgentState::Idle),
        // A pulse that is deliberately old: the agent was running this morning
        // and has stopped answering. That is the row a manager should look at
        // first, and it is only visible if the demo carries one - "no pulse at
        // all" reads as `unknown`, which says nothing about the person.
        Pattern::Long | Pattern::Fading => Some(AgentState::Working),
        // Silent and Never never sent one, which is `unknown`.
        _ => None,
    };
    // Zero for the live ones; well past the threshold for the two that have
    // stopped. Recorded on the row rather than recomputed, so the refresh
    // knows which is which - a pulse that merely aged is indistinguishable
    // from one seeded old.
    let age = state.map(|_| {
        if matches!(pattern, Pattern::Long | Pattern::Fading) {
            STALE_PULSE_AGE_SECONDS
        } else {
            0
        }
    });
    (state, age)
}

/// Gives the demo's agents their pulses if they have none.
///
/// The upgrade path. A demo seeded before this milestone has agents but no
/// pulses, and bumping the image does not re-seed - so its dashboard would
/// show twelve rows of "unknown" and none of the live column the version was
/// released for. Found by deploying to the project's own demo stand, not by a
/// test: every test seeds from empty, where the question cannot arise.
///
/// Idempotent, and it never overwrites a pulse that exists: an agent that has
/// reported - including a real kasl pointed at the demo - is left alone. The
/// people are matched by the email the generator assigns, so nothing outside
/// the fictional team is touched.
pub async fn ensure_pulses(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let mut given = 0;
    for person in TEAM.iter() {
        let Some(pattern) = person.pattern else { continue };
        let (Some(state), age) = pulse_for(pattern) else { continue };
        let updated = sqlx::query(
            "UPDATE agents SET heartbeat_state = $2, demo_pulse_age_seconds = $3,
                    heartbeat_at = now() - coalesce($3, 0) * interval '1 second',
                    heartbeat_received_at = now() - coalesce($3, 0) * interval '1 second'
             FROM users u
             WHERE agents.user_id = u.id AND lower(u.email) = lower($1)
               AND agents.heartbeat_state IS NULL AND agents.revoked_at IS NULL",
        )
        .bind(person.email())
        .bind(state)
        .bind(age)
        .execute(pool)
        .await?;
        given += updated.rows_affected();
    }
    Ok(given)
}

/// Re-stamps the demo's seeded pulses so they stay fresh.
///
/// The demo is seeded once, but a pulse is believed for three minutes - so
/// without this every live row on the demo would turn "offline" while the
/// first visitor was still reading the page, and the milestone would be
/// invisible on the one installation built to show it off.
///
/// Only rows that already carry a state are touched: which people are working,
/// paused or idle was decided at seed time and stays decided, and an agent
/// seeded silent must keep looking silent. Returns how many were re-stamped.
///
/// Only the pulses that were fresh when they were written are pulled forward.
/// Two of the demo's agents are seeded deliberately stale - they are the "this
/// machine has stopped answering" row a manager should look at first - and a
/// refresh that pulled every stamp up to `now()` would quietly heal them,
/// leaving the dashboard with no offline row to show. They are held at a fixed
/// age instead, so they stay offline for as long as the demo runs.
///
/// Nothing like this runs on a real installation - there a pulse means an
/// agent sent one, and a server that invented them would be lying about the
/// only thing this endpoint is for.
pub async fn refresh_pulses(pool: &PgPool) -> Result<u64, sqlx::Error> {
    // Each row is re-stamped to the age the seed chose for it, read off the
    // row itself. Only agents the demo gave an age are touched, so an agent a
    // visitor pointed at the demo keeps whatever pulse it actually sent.
    let updated = sqlx::query(
        "UPDATE agents
         SET heartbeat_at = now() - demo_pulse_age_seconds * interval '1 second',
             heartbeat_received_at = now() - demo_pulse_age_seconds * interval '1 second'
         WHERE heartbeat_state IS NOT NULL AND demo_pulse_age_seconds IS NOT NULL AND revoked_at IS NULL",
    )
    .execute(pool)
    .await?;
    Ok(updated.rows_affected())
}

/// How old the demo's stopped agents are kept.
///
/// Comfortably past the staleness threshold, and stable across refreshes, so
/// "this machine stopped answering" stays on the dashboard rather than healing
/// itself a minute after the visitor arrives.
const STALE_PULSE_AGE_SECONDS: i32 = (heartbeat::STALE_AFTER_SECONDS * 4) as i32;

/// Keeps the demo's pulses fresh for as long as the server runs.
///
/// Started only when the installation is a demo. Re-stamps at half the
/// staleness threshold, so a slow tick cannot let the dashboard flicker
/// through "offline" between two refreshes.
pub fn keep_pulses_fresh(pool: PgPool) {
    let period = std::time::Duration::from_secs((heartbeat::STALE_AFTER_SECONDS / 2).max(1) as u64);
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(period);
        // The first tick fires immediately and would re-stamp what `seed` just
        // wrote; skipping it costs nothing and keeps the log quiet.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if let Err(error) = refresh_pulses(&pool).await {
                // Worth a line, not worth stopping for: the dashboard degrades
                // to "offline", which is at least an honest reading of a
                // server that cannot reach its database.
                tracing::warn!(%error, "failed to refresh the demo pulses");
            }
        }
    });
}

/// Eight weeks of one person's days, ending yesterday.
///
/// Deterministic: the same person on the same date produces the same days.
/// `index` seeds the generator so people do not share a stream and reordering
/// the team table does not reshape everyone's history.
fn days_for(person: &Person, pattern: Pattern, index: u64, now: DateTime<Utc>) -> Vec<AgentDay> {
    if pattern == Pattern::Never {
        return Vec::new();
    }

    let mut rng = Rng::new(index);
    let now_local = now.with_timezone(&person.offset()).naive_local();
    let today = now_local.date();
    let first = today - Duration::weeks(WEEKS);
    let pool = task_pool(person.department.unwrap_or("Support"));

    // Tasks not yet finished, carried into the next day the way kasl carries
    // them: same group id, higher completeness.
    let mut carried: Vec<(i32, &'static str, i16)> = Vec::new();
    let mut next_task_id = 1;
    let mut days = Vec::new();

    for offset in 0..(today - first).num_days() {
        let date = first + Duration::days(offset);

        if matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
            continue;
        }
        // A sick day now and then: the dashboard should show a gap.
        if rng.chance(4) {
            continue;
        }
        if pattern == Pattern::Silent && date >= today - Duration::days(7) {
            continue;
        }

        let week = ((date - first).num_days() / 7) as f64;
        let (start_minute, span_minutes) = match pattern {
            Pattern::Long => (rng.range(8 * 60, 8 * 60 + 30), rng.range(10 * 60, 11 * 60 + 15)),
            Pattern::Fading => (rng.range(9 * 60, 9 * 60 + 45), (8.5 * 60.0 - week * 0.45 * 60.0) as i64 + rng.range(-15, 15)),
            _ => (rng.range(8 * 60 + 45, 9 * 60 + 30), rng.range(7 * 60 + 45, 8 * 60 + 45)),
        };
        let start = date.and_time(minutes(start_minute));
        let end = start + Duration::minutes(span_minutes);

        let pauses = pauses_for(&mut rng, pattern, start, span_minutes);
        let tasks = tasks_for(&mut rng, pool, &mut carried, &mut next_task_id, end);

        days.push(AgentDay {
            date,
            start,
            end: Some(end),
            pauses,
            tasks,
        });
    }

    if pattern == Pattern::Open {
        // Started three hours ago, or at midnight if the day is younger than
        // that: an open day's start cannot lie on yesterday's date, which has
        // its own row.
        let start = (now_local - Duration::hours(3)).max(today.and_time(NaiveTime::MIN));
        let elapsed = (now_local - start).num_minutes();
        let mut pauses = Vec::new();
        if elapsed > 90 {
            let pause_start = start + Duration::minutes(elapsed / 2);
            pauses.push(AgentPause {
                start: pause_start,
                end: Some(pause_start + Duration::minutes(11)),
                duration_seconds: Some(11 * 60),
                manual: false,
                reason: None,
            });
        }
        let name = carried.first().map(|(_, name, _)| *name).unwrap_or(pool[0]);
        let group = carried.first().map(|(group, _, _)| *group).unwrap_or(next_task_id);
        days.push(AgentDay {
            date: today,
            start,
            end: None,
            pauses,
            tasks: vec![AgentTask {
                agent_task_id: next_task_id,
                agent_group_id: group,
                recorded_at: now_local - Duration::minutes(elapsed.min(20)),
                name: name.to_string(),
                comment: None,
                completeness: 40,
            }],
        });
    }

    days
}

/// The interruptions of one day.
///
/// Placed on hourly slots so they never overlap: each starts within the first
/// half hour of its slot and lasts under half an hour. Lunch sits on the
/// fourth hour for everyone; whether it is a detected absence or a break the
/// person entered depends on the pattern.
fn pauses_for(rng: &mut Rng, pattern: Pattern, start: NaiveDateTime, span_minutes: i64) -> Vec<AgentPause> {
    let manual = pattern == Pattern::Breaks;
    let hours = span_minutes / 60;
    let mut pauses = Vec::new();

    let lunch = start + Duration::hours(4) + Duration::minutes(rng.range(0, 15));
    pauses.push(pause(lunch, rng.range(28, 40), manual, manual.then_some("Lunch")));

    let idle_count = match pattern {
        Pattern::Long => rng.range(1, 2),
        Pattern::Breaks => rng.range(1, 3),
        _ => rng.range(2, 3),
    };
    let mut slots: Vec<i64> = (1..hours).filter(|slot| *slot != 4).collect();
    for _ in 0..idle_count {
        if slots.is_empty() {
            break;
        }
        let slot = slots.remove(rng.range(0, slots.len() as i64 - 1) as usize);
        let at = start + Duration::hours(slot) + Duration::minutes(rng.range(0, 30));
        let reason = manual.then(|| *rng.pick(&BREAK_REASONS[1..]));
        pauses.push(pause(at, rng.range(5, 25), manual, reason));
    }

    pauses.sort_by_key(|pause| pause.start);
    pauses
}

fn pause(start: NaiveDateTime, minutes: i64, manual: bool, reason: Option<&str>) -> AgentPause {
    AgentPause {
        start,
        end: Some(start + Duration::minutes(minutes)),
        duration_seconds: Some((minutes * 60) as i32),
        manual,
        reason: reason.map(str::to_string),
    }
}

/// The tasks logged at the end of one day.
///
/// About half the time a task continues from the day before, so the history
/// shows work carried across days the way kasl records it; the rest are new.
/// Unfinished ones are carried forward, two at most.
fn tasks_for(rng: &mut Rng, pool: &[&'static str], carried: &mut Vec<(i32, &'static str, i16)>, next_id: &mut i32, end: NaiveDateTime) -> Vec<AgentTask> {
    let count = rng.range(2, 4);
    let mut tasks = Vec::new();
    let mut still_open = Vec::new();

    for _ in 0..count {
        let continued = !carried.is_empty() && rng.chance(55);
        let (group, name, completeness) = if continued {
            let (group, name, done) = carried.remove(0);
            (group, name, (done + rng.range(20, 50) as i16).min(100))
        } else {
            let name = *rng.pick(pool);
            (*next_id, name, [20i16, 40, 60, 80, 100][rng.range(0, 4) as usize])
        };
        let id = *next_id;
        *next_id += 1;

        tasks.push(AgentTask {
            agent_task_id: id,
            agent_group_id: group,
            recorded_at: end - Duration::minutes(rng.range(2, 15)),
            name: name.to_string(),
            comment: None,
            completeness,
        });
        if completeness < 100 && still_open.len() < 2 {
            still_open.push((group, name, completeness));
        }
    }

    *carried = still_open;
    tasks
}

fn minutes(since_midnight: i64) -> NaiveTime {
    NaiveTime::from_num_seconds_from_midnight_opt((since_midnight * 60) as u32, 0).expect("a minute of the day")
}

/// A small deterministic generator (xorshift64*).
///
/// Not `rand`: its output for a given seed is not promised to stay the same
/// across versions, and the point of seeding is that the demo looks the same
/// after a dependency update as before it.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // Mixed so that neighbouring seeds do not produce neighbouring streams;
        // the `| 1` keeps zero - the one state xorshift never leaves - out.
        Self((seed + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A value in `low..=high`.
    fn range(&mut self, low: i64, high: i64) -> i64 {
        debug_assert!(low <= high);
        low + (self.next() % (high - low + 1) as u64) as i64
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.range(0, items.len() as i64 - 1) as usize]
    }
}

/// What `GET /api/v1/demo/accounts` answers on a demo.
#[derive(Debug, Serialize)]
struct Accounts {
    password: &'static str,
    accounts: Vec<Account>,
}

/// The accounts a visitor may sign in as, and their password.
///
/// Unauthenticated on purpose - it is what the login screen shows before
/// anyone is signed in - and answered only where the database says this is a
/// demo, so a real installation with a real team never lists anybody here.
pub async fn accounts(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let demo: bool = sqlx::query_scalar("SELECT demo FROM settings WHERE singleton").fetch_one(&state.pool).await?;
    if !demo {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "this server is not a demo"));
    }
    Ok(Json(Accounts {
        password: PASSWORD,
        accounts: showcase(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(date: &str, time: &str) -> DateTime<Utc> {
        format!("{date}T{time}Z").parse().expect("a test timestamp")
    }

    fn person(pattern: Pattern) -> &'static Person {
        TEAM.iter().find(|person| person.pattern == Some(pattern)).expect("every pattern has a person")
    }

    #[test]
    fn every_department_names_a_manager_who_is_in_the_team() {
        for department in &DEPARTMENTS {
            let manager = TEAM.iter().find(|person| person.email().starts_with(department.manager));
            let manager = manager.unwrap_or_else(|| panic!("{} names nobody in the team", department.name));
            assert_eq!(manager.role, UserRole::Manager, "{} is run by a {:?}", department.name, manager.role);
            assert_eq!(manager.department, Some(department.name), "a manager belongs to the department they run");
        }
    }

    #[test]
    fn the_team_has_one_of_every_role_and_every_pattern() {
        for role in [UserRole::Admin, UserRole::Manager, UserRole::Employee] {
            assert!(TEAM.iter().any(|person| person.role == role), "nobody is a {role:?}");
        }
        for pattern in [
            Pattern::Steady,
            Pattern::Long,
            Pattern::Fading,
            Pattern::Breaks,
            Pattern::Open,
            Pattern::Silent,
            Pattern::Never,
        ] {
            assert!(TEAM.iter().any(|person| person.pattern == Some(pattern)), "nobody is {pattern:?}");
        }
        assert_eq!(showcase().len(), 3, "one account of each role to try");
        assert_eq!(showcase()[0].role, UserRole::Manager, "the manager's dashboard is what the demo is for");
    }

    #[test]
    fn emails_are_under_a_reserved_domain() {
        // The names are invented; the domain guarantees the addresses are too.
        for person in &TEAM {
            assert!(person.email().ends_with("@example.com"), "{}", person.email());
        }
        let mut emails: Vec<String> = TEAM.iter().map(Person::email).collect();
        emails.dedup();
        assert_eq!(emails.len(), TEAM.len(), "two people share an address");
    }

    #[test]
    fn the_history_is_the_same_for_the_same_today() {
        // A screenshot taken from one demo must be reproducible on another
        // started the same day.
        let now = at("2026-08-29", "10:00:00");
        let person = person(Pattern::Steady);
        let one = days_for(person, Pattern::Steady, 4, now);
        let two = days_for(person, Pattern::Steady, 4, now);
        assert_eq!(one.len(), two.len());
        for (a, b) in one.iter().zip(&two) {
            assert_eq!(
                (a.date, a.start, a.end, a.pauses.len(), a.tasks.len()),
                (b.date, b.start, b.end, b.pauses.len(), b.tasks.len())
            );
        }
        assert!(
            one.len() >= 35,
            "eight weeks of weekdays, minus the odd sick day, is at least 35 days; got {}",
            one.len()
        );
    }

    #[test]
    fn days_end_yesterday_and_skip_weekends() {
        let now = at("2026-08-29", "10:00:00");
        let person = person(Pattern::Steady);
        let days = days_for(person, Pattern::Steady, 4, now);
        let today = now.with_timezone(&person.offset()).date_naive();
        for day in &days {
            assert!(day.date < today, "{} is not in the past", day.date);
            assert!(!matches!(day.date.weekday(), Weekday::Sat | Weekday::Sun), "{} is a weekend", day.date);
            assert!(day.end.is_some(), "every past day is closed");
            let end = day.end.unwrap();
            for pause in &day.pauses {
                assert!(pause.start > day.start && pause.end.unwrap() < end, "a pause lies inside its day");
            }
            for pair in day.pauses.windows(2) {
                assert!(pair[0].end.unwrap() <= pair[1].start, "pauses do not overlap on {}", day.date);
            }
        }
    }

    #[test]
    fn the_open_day_is_today_and_still_running() {
        let now = at("2026-08-29", "14:00:00");
        let person = person(Pattern::Open);
        let days = days_for(person, Pattern::Open, 7, now);
        let today = now.with_timezone(&person.offset()).date_naive();
        let open = days.iter().find(|day| day.end.is_none()).expect("one day is open");
        assert_eq!(open.date, today);
        assert!(open.start <= now.with_timezone(&person.offset()).naive_local());
        assert_eq!(days.iter().filter(|day| day.end.is_none()).count(), 1);
    }

    #[test]
    fn an_open_day_started_after_midnight_does_not_reach_into_yesterday() {
        // 01:00 local: three hours ago is yesterday, which has its own row.
        let person = person(Pattern::Open);
        let now = at("2026-08-29", "04:00:00"); // 01:00 at UTC-3
        let days = days_for(person, Pattern::Open, 7, now);
        let open = days.iter().find(|day| day.end.is_none()).unwrap();
        assert_eq!(open.start, open.date.and_time(NaiveTime::MIN));
    }

    #[test]
    fn the_silent_agent_stops_a_week_before_today() {
        let now = at("2026-08-29", "10:00:00");
        let person = person(Pattern::Silent);
        let days = days_for(person, Pattern::Silent, 9, now);
        let today = now.with_timezone(&person.offset()).date_naive();
        assert!(!days.is_empty());
        assert!(days.iter().all(|day| day.date < today - Duration::days(7)), "nothing in the last week");
    }

    #[test]
    fn the_fading_hours_actually_fade() {
        let now = at("2026-08-29", "10:00:00");
        let person = person(Pattern::Fading);
        let days = days_for(person, Pattern::Fading, 6, now);
        let span = |day: &AgentDay| (day.end.unwrap() - day.start).num_minutes();
        let first_week: Vec<i64> = days.iter().take(5).map(span).collect();
        let last_week: Vec<i64> = days.iter().rev().take(5).map(span).collect();
        let average = |week: &[i64]| week.iter().sum::<i64>() / week.len() as i64;
        assert!(
            average(&first_week) - average(&last_week) > 120,
            "the last week should be hours shorter than the first: {first_week:?} vs {last_week:?}"
        );
    }

    #[test]
    fn tasks_carry_across_days_under_one_group() {
        let now = at("2026-08-29", "10:00:00");
        let person = person(Pattern::Steady);
        let days = days_for(person, Pattern::Steady, 4, now);
        let mut ids = std::collections::HashSet::new();
        let mut carried = 0;
        for day in &days {
            for task in &day.tasks {
                assert!(ids.insert(task.agent_task_id), "agent_task_id {} repeats", task.agent_task_id);
                if task.agent_group_id != task.agent_task_id {
                    carried += 1;
                }
            }
        }
        assert!(carried > 0, "some work should span more than one day");
    }

    #[test]
    fn the_never_pattern_has_no_days() {
        let person = person(Pattern::Never);
        assert!(days_for(person, Pattern::Never, 11, at("2026-08-29", "10:00:00")).is_empty());
    }

    #[test]
    fn the_generator_stays_in_range() {
        let mut rng = Rng::new(3);
        for _ in 0..1000 {
            let value = rng.range(5, 7);
            assert!((5..=7).contains(&value));
        }
        let mut heads = 0;
        for _ in 0..1000 {
            if rng.chance(50) {
                heads += 1;
            }
        }
        assert!((350..=650).contains(&heads), "a 50% chance came up {heads} times in 1000");
    }
}
