//! Alerts: what the server noticed now, and what somebody did about it.
//!
//! Every view before this one waits to be opened. The signals say where to
//! look, which is a real advance over a table of totals - and they still say
//! it on a page, in whole weeks, to whoever happens to visit. The case this
//! milestone exists for is the one where waiting is the failure: an agent that
//! died on Monday morning, a day left open over a weekend, somebody's eleventh
//! hour. None of those improve by being discovered on Thursday.
//!
//! **An alert is a stored observation, and a signal is not stored at all.**
//! That difference is the whole design, and it is not an inconsistency. A
//! signal is a function of the days already in the database, so a table of
//! signals would be a second copy of a derived fact (ADR 0016). An alert
//! carries two things no day can produce:
//!
//! * **when the condition began.** The absence of rows has no timestamp. A
//!   recomputation can say a thing is true; only a row says since when.
//! * **what a person decided about it.** A manager who looked and concluded it
//!   was a holiday needs it to stop shouting. An alert that cannot be answered
//!   trains people to ignore the whole column, which is the lesson `unknown`
//!   taught in ADR 0014.
//!
//! So the rule is still computed from the days, every sweep. What is written
//! down is the event of having noticed - and the sweep **reconciles**, rather
//! than inserting: it builds what ought to be open right now, compares it with
//! what is open, and moves the difference. That is why a week of silence is
//! one row and not seven, and why an agent coming back closes its own alert
//! without anybody clicking anything.
//!
//! The three rules, and why each one can be stated honestly only now:
//!
//! * `no_agent_data` - nothing has arrived from any of this person's machines
//!   for longer than the installation's threshold. In hours, about *now*,
//!   which is what makes it a different object from the `no_data` signal: that
//!   one measures a person's own weekly rhythm and cannot speak before the
//!   week is complete.
//! * `overwork` - a finished day ran past what that person owed it, by a
//!   share of their own norm. Before the production calendar this could only
//!   have been a number of hours invented here, which is this product
//!   asserting what a working day is on someone else's team - exactly what
//!   ADR 0016 refused to do. With a norm and a rate it is arithmetic
//!   (ADR 0017).
//! * `day_not_closed` - a day is still open long after any day plausibly runs.
//!   Usually kasl left running overnight, and the day it will eventually
//!   produce is wrong in a way that quietly poisons a week's total.
//!
//! Delivery here is in-app, and deliberately: a webhook into a chat at 3 a.m.
//! is a different product decision with its own milestone (v0.23), and the row
//! this module writes is precisely what that one will ship outward. Delivery
//! gets added to the record; it does not replace it.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use rust_decimal::{Decimal, prelude::ToPrimitive};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    admin::{VISIBLE_USERS, require_manager_or_admin},
    app::AppState,
    audit,
    calendar::{Calendar, WorkdayKind},
    error::ApiError,
    login::CurrentUser,
    model::UserRole,
};

/// Seconds in an hour.
const SECONDS_PER_HOUR: i64 = 3600;

/// Which rule noticed something.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "alert_rule", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum AlertRule {
    /// Nothing has arrived from this person's agents for longer than the
    /// installation allows.
    NoAgentData,
    /// A finished day ran well past that person's own norm for it.
    Overwork,
    /// A day is still open long after any day plausibly runs.
    DayNotClosed,
}

impl AlertRule {
    /// Which alerts a feed shows first. Lower sorts earlier.
    ///
    /// Silence outranks the rest for the reason it does among the signals: an
    /// agent that stopped reporting makes every other number about that person
    /// untrustworthy, including the two below.
    fn severity(self) -> u8 {
        match self {
            Self::NoAgentData => 0,
            Self::DayNotClosed => 1,
            Self::Overwork => 2,
        }
    }
}

/// Where an alert stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "alert_state", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum AlertState {
    /// Still true, and still unattended.
    Open,
    /// A person looked and decided it needs no action. The condition may well
    /// still hold - the alert is answered, not gone.
    Acknowledged,
    /// The condition stopped being true on its own, and nobody had to do
    /// anything. Distinct from `acknowledged` on purpose: how much of what the
    /// server shouted about was real is the one question worth asking of this
    /// table later, and a single "closed" flag would throw the answer away.
    Resolved,
}

/// The thresholds this installation speaks at.
///
/// Settings, unlike the signal thresholds, which ADR 0016 deliberately fixed
/// in code. The distinction is who is interrupted. A signal is read by whoever
/// opened the page, and a sensitivity slider there is a knob on an opinion. An
/// alert interrupts somebody, and how much silence is worth interrupting over
/// genuinely differs between a team in one timezone and a team across four.
///
/// What is not configurable is the set of rules - that stays a choice, not an
/// operator's to invent.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, sqlx::FromRow)]
pub struct Thresholds {
    /// Hours of silence from every one of a person's agents before it is an
    /// alert.
    pub alert_silence_hours: i32,
    /// How far past their own norm a finished day has to run, as a share of
    /// that norm. A share rather than a number of hours, so it means the same
    /// thing for somebody on half time.
    pub alert_overwork_factor: Decimal,
    /// Hours a day may stay open before the server says so.
    pub alert_open_day_hours: i32,
}

impl Thresholds {
    pub async fn load(pool: &PgPool) -> Result<Self, ApiError> {
        Ok(
            sqlx::query_as("SELECT alert_silence_hours, alert_overwork_factor, alert_open_day_hours FROM settings WHERE singleton")
                .fetch_one(pool)
                .await?,
        )
    }

    fn silence_seconds(&self) -> i64 {
        i64::from(self.alert_silence_hours) * SECONDS_PER_HOUR
    }

    fn open_day_seconds(&self) -> i64 {
        i64::from(self.alert_open_day_hours) * SECONDS_PER_HOUR
    }
}

/// What the rules are evaluated against, for one person, at one moment.
///
/// A plain struct assembled by the sweep's queries rather than the rows
/// themselves, so every rule below is a function of values and can be tested
/// without a database. The arithmetic is the part most worth testing, and
/// logic inside a query can only be tested through one - the same reason the
/// signals do their statistics in Rust.
#[derive(Debug, Clone)]
pub struct Observation {
    pub user_id: Uuid,
    /// When anything last arrived from any of this person's live agents: the
    /// later of a request that used the token and a pulse. `None` for somebody
    /// whose agent has never said anything at all.
    ///
    /// **Both, and the freshest wins.** `agents.last_seen_at` alone is what a
    /// first version of this rule read, and a live run against the demo showed
    /// what that costs: nine of twelve people flagged as silent, six of them
    /// having pulsed seconds earlier. The two stamps answer different
    /// questions - "the token was used" and "kasl is watching a person work"
    /// (ADR 0014) - and an agent can go a long time doing only the second: a
    /// machine whose employee is on holiday pulses `idle` all week and uploads
    /// nothing at all. Reading one of them is how a rule about *silence*
    /// alerts on somebody who is plainly talking.
    pub last_seen_at: Option<DateTime<Utc>>,
    /// Whether this person has any live agent token. Somebody with none is not
    /// silent - they were never asked to speak.
    pub has_live_agent: bool,
    /// The finished day with the most seconds worked in the window the sweep
    /// looks at, as `(date, worked_seconds, kind)`.
    pub longest_finished_day: Option<(NaiveDate, i64, WorkdayKind)>,
    /// A day still open, as `(date, started_at)`. At most one: the schema
    /// allows a person one day per date, and a second open day would be a much
    /// louder problem than this rule.
    pub open_day: Option<(NaiveDate, DateTime<Utc>)>,
    /// This person's share of a full day.
    pub work_rate: Decimal,
}

/// A condition the sweep found true: the rule, and the figures it fired on.
///
/// Not an alert yet. Whether it becomes a row depends on what is already open,
/// which is the sweep's business rather than the rule's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub user_id: Uuid,
    pub rule: AlertRule,
    /// What was measured, in seconds.
    pub observed_seconds: i64,
    /// What it was measured against, in seconds, where there is a second
    /// figure to show. Both travel to the screen because a percentage cannot
    /// be un-divided: "11 h against a norm of 8" and "138%" are not the same
    /// sentence, and only the first lets a reader disagree with it (ADR 0017).
    pub against_seconds: Option<i64>,
    /// The date it is about, for the rules that are about a day.
    pub subject_date: Option<NaiveDate>,
}

/// Every rule, applied to one person.
///
/// The whole of what this server is willing to interrupt somebody about, in
/// one function, so the set can be read in one place and a fourth rule has an
/// obvious home.
pub fn findings(observation: &Observation, calendar: &Calendar, standard_hours: Decimal, thresholds: &Thresholds, now: DateTime<Utc>) -> Vec<Finding> {
    let mut found = Vec::new();

    if let Some(silent_for) = silent_for(observation, thresholds, now) {
        found.push(Finding {
            user_id: observation.user_id,
            rule: AlertRule::NoAgentData,
            observed_seconds: silent_for,
            against_seconds: Some(thresholds.silence_seconds()),
            subject_date: None,
        });
    }

    if let Some((date, worked, norm)) = overworked(observation, calendar, standard_hours, thresholds) {
        found.push(Finding {
            user_id: observation.user_id,
            rule: AlertRule::Overwork,
            observed_seconds: worked,
            against_seconds: Some(norm),
            subject_date: Some(date),
        });
    }

    if let Some((date, open_for)) = open_too_long(observation, thresholds, now) {
        found.push(Finding {
            user_id: observation.user_id,
            rule: AlertRule::DayNotClosed,
            observed_seconds: open_for,
            against_seconds: Some(thresholds.open_day_seconds()),
            subject_date: Some(date),
        });
    }

    found
}

/// How long this person's agents have been silent, when that is longer than
/// the installation allows.
///
/// Somebody with no live agent is not silent - nothing was ever asked of them,
/// and an installation that has not finished handing out tokens would otherwise
/// alert about every account on its first day. The dashboard already says
/// "no agents" in words, which is a different and more useful sentence.
///
/// Somebody who has an agent and has never used it is silent from the moment
/// the token was issued - but this rule cannot see that date, and inventing
/// one would be worse than the `no_data` signal's honest silence. `None`, and
/// the team table's "never reported" keeps that case.
fn silent_for(observation: &Observation, thresholds: &Thresholds, now: DateTime<Utc>) -> Option<i64> {
    if !observation.has_live_agent {
        return None;
    }
    let last_seen = observation.last_seen_at?;
    let silent = (now - last_seen).num_seconds();
    (silent >= thresholds.silence_seconds()).then_some(silent)
}

/// The finished day that ran furthest past its norm, when one did, as
/// `(date, worked, norm)`.
///
/// Against that person's own norm for that date, so a short day before a
/// holiday is a lower bar and a half-time employee is measured against half a
/// day. An installation-wide "more than ten hours" would call a part-timer's
/// doubled day ordinary and never mention it.
///
/// A day of leave or illness is skipped rather than compared: its norm is
/// zero, so any work at all on it would exceed the norm by an infinite share,
/// and "you worked on your holiday" is a fact this product has no business
/// raising with a manager. That is between the employee and their own screen.
fn overworked(observation: &Observation, calendar: &Calendar, standard_hours: Decimal, thresholds: &Thresholds) -> Option<(NaiveDate, i64, i64)> {
    let (date, worked, kind) = observation.longest_finished_day?;
    if !kind.owes_the_norm() {
        return None;
    }

    let norm = calendar.norm_seconds(date, standard_hours, observation.work_rate);
    // A date the calendar says is not worked - a weekend, a holiday - has a
    // norm of zero and no factor of it is anything. Somebody working a
    // Saturday is worth noticing, and this is not the rule that notices it:
    // there is no norm to be a multiple of, so a threshold here would be a
    // number invented after all.
    if norm <= 0 {
        return None;
    }

    let threshold = (Decimal::from(norm) * thresholds.alert_overwork_factor).round().to_i64()?;
    (worked >= threshold).then_some((date, worked, norm))
}

/// How long a day has been open, when that is longer than any day runs.
///
/// Measured from the day's start on the wall clock, not from its norm: an open
/// day has no total yet - there is nothing to compare with a norm - and what
/// is wrong with it is simply elapsed time.
fn open_too_long(observation: &Observation, thresholds: &Thresholds, now: DateTime<Utc>) -> Option<(NaiveDate, i64)> {
    let (date, started_at) = observation.open_day?;
    let open_for = (now - started_at).num_seconds();
    (open_for >= thresholds.open_day_seconds()).then_some((date, open_for))
}

// The sweep -------------------------------------------------------------------

/// How far back the sweep looks for a day to judge.
///
/// A fortnight: long enough that a day filed late is still examined, short
/// enough that the sweep is not re-reading a quarter every few minutes. An
/// alert about a day from last month would be archaeology, not an alert.
const LOOKBACK_DAYS: i64 = 14;

/// What one sweep did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Swept {
    /// Conditions that were not open before: new rows.
    pub raised: u64,
    /// Open alerts whose condition stopped being true: closed by the server.
    pub resolved: u64,
    /// Conditions that held and produced no new row: already open, or already
    /// answered by somebody. Reported because zero raised and zero resolved is
    /// the same output as a sweep that found nothing at all, and only one of
    /// those is quiet good news.
    ///
    /// The two cases are counted together on purpose - this is a log line, and
    /// what it has to say is "the sweep ran and the world was as expected".
    /// Which alerts are open and which were answered is a question for the
    /// feed, where the rows are.
    pub unchanged: u64,
}

/// Looks at everyone, and moves the alerts to match what is true now.
///
/// Reconciliation rather than insertion, which is what keeps a fortnight of
/// silence one row instead of a row per sweep. Three outcomes per person and
/// rule: the condition holds and nothing is open (raise), it holds and
/// something is open (leave it - notably **without** rewriting its figures, so
/// "quiet since Monday, 9 h" does not silently become "quiet since Monday,
/// 400 h"; the sentence an alert makes is the one it made when it fired), or
/// nothing holds and something is open (resolve).
///
/// An acknowledged alert is not touched by any of this. A person answered it,
/// and the server re-raising it on the next sweep is the exact behaviour that
/// makes people stop reading alerts. It comes back only if the condition
/// resolves and later becomes true again, which is a genuinely new event.
pub async fn sweep(pool: &PgPool, now: DateTime<Utc>) -> Result<Swept, ApiError> {
    let thresholds = Thresholds::load(pool).await?;
    let standard_hours: Decimal = sqlx::query_scalar("SELECT standard_hours FROM settings WHERE singleton")
        .fetch_one(pool)
        .await?;

    let today = now.date_naive();
    let from = today - TimeDelta::days(LOOKBACK_DAYS);
    let calendar = Calendar::load(pool, from, today).await?;

    let observations = observe(pool, from, today).await?;

    // What a person has already answered. Not expressible as an `ON CONFLICT`
    // clause: the unique index is on the *open* rows alone - deliberately, so
    // a silence in March does not block a silence in July - so an acknowledged
    // row conflicts with nothing and an insert would sail straight past it.
    //
    // An acknowledgement stands until its condition resolves. The alternative
    // was found by a test rather than by reading: the server re-raised what a
    // manager had just dismissed on the very next sweep, five minutes later,
    // which is precisely the behaviour that makes people stop reading alerts.
    // `resolved_at IS NULL` is what makes the suppression last exactly as long
    // as the condition did: once a sweep has seen the condition go away, the
    // stamp is set and this row stops standing in the way of the next one.
    let acknowledged: Vec<(Uuid, AlertRule)> = sqlx::query_as("SELECT user_id, rule FROM alerts WHERE state = 'acknowledged' AND resolved_at IS NULL")
        .fetch_all(pool)
        .await?;

    // Computed once and used by both halves of the reconciliation. Running
    // the rules twice would be cheap and would also be two answers to "what is
    // true now" that nothing forces to agree - and a sweep whose two halves
    // disagreed would raise a row and resolve it in the same pass, forever.
    let found: Vec<Finding> = observations
        .iter()
        .flat_map(|observation| findings(observation, &calendar, standard_hours, &thresholds, now))
        .collect();

    let mut swept = Swept::default();
    for finding in &found {
        if acknowledged.contains(&(finding.user_id, finding.rule)) {
            swept.unchanged += 1;
            continue;
        }
        // `ON CONFLICT DO NOTHING` against the partial unique index: the
        // index is what makes "one open alert per person per rule" true
        // even if two sweeps overlap, and this is how the loser of that
        // race finds out without failing.
        let inserted = sqlx::query(
            "INSERT INTO alerts (user_id, rule, observed_seconds, against_seconds, subject_date, fired_at)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (user_id, rule) WHERE state = 'open' DO NOTHING",
        )
        .bind(finding.user_id)
        .bind(finding.rule)
        .bind(finding.observed_seconds)
        .bind(finding.against_seconds)
        .bind(finding.subject_date)
        .bind(now)
        .execute(pool)
        .await?;

        if inserted.rows_affected() > 0 {
            swept.raised += 1;
            tracing::info!(user = %finding.user_id, rule = ?finding.rule, "raised an alert");
        } else {
            swept.unchanged += 1;
        }
    }

    let still_true: Vec<(Uuid, AlertRule)> = found.iter().map(|finding| (finding.user_id, finding.rule)).collect();

    // A person the sweep did not observe at all - an account deactivated since
    // the alert was raised - has no findings, so everything open about them
    // resolves. That is the right answer: the condition is no longer being
    // observed to hold, and an alert nobody can act on any more is noise.
    //
    // The acknowledged rows are swept too, and that is not a detail. An
    // acknowledgement suppresses the next raise for as long as the condition
    // lasts; if it never ended, the suppression would be permanent, and a
    // person quiet again next month would go unmentioned because somebody
    // dismissed last month's silence. Ending it here is what makes the
    // return of a condition a genuinely new event rather than a lost one.
    let standing: Vec<(Uuid, Uuid, AlertRule)> = sqlx::query_as("SELECT id, user_id, rule FROM alerts WHERE state IN ('open', 'acknowledged')")
        .fetch_all(pool)
        .await?;

    for (id, user_id, rule) in standing {
        if still_true.contains(&(user_id, rule)) {
            continue;
        }
        let closed = sqlx::query("UPDATE alerts SET state = 'resolved', resolved_at = $2 WHERE id = $1 AND state = 'open'")
            .bind(id)
            .bind(now)
            .execute(pool)
            .await?;

        if closed.rows_affected() > 0 {
            swept.resolved += 1;
            tracing::info!(user = %user_id, rule = ?rule, "an alert resolved itself");
            continue;
        }

        // An acknowledged one keeps its state - it was answered, and rewriting
        // that to `resolved` would erase the fact that a person looked. Only
        // the stamp is added, which is what a later sweep reads to know the
        // suppression is spent.
        sqlx::query("UPDATE alerts SET resolved_at = $2 WHERE id = $1 AND state = 'acknowledged' AND resolved_at IS NULL")
            .bind(id)
            .bind(now)
            .execute(pool)
            .await?;
    }

    Ok(swept)
}

/// Gathers what the rules need, for everyone who could have an alert.
///
/// Everyone active, not only the people some particular manager may see: the
/// sweep writes the record, and who is allowed to *read* a given row is
/// decided when the feed is asked for. Filtering here would make an alert's
/// existence depend on who happened to trigger the sweep.
async fn observe(pool: &PgPool, from: NaiveDate, to: NaiveDate) -> Result<Vec<Observation>, ApiError> {
    let rows: Vec<ObservationRow> = sqlx::query_as(
        "SELECT u.id AS user_id,
                u.work_rate,
                (SELECT max(greatest(a.last_seen_at, a.heartbeat_received_at))
                 FROM agents a WHERE a.user_id = u.id AND a.revoked_at IS NULL) AS last_seen_at,
                EXISTS (SELECT 1 FROM agents a WHERE a.user_id = u.id AND a.revoked_at IS NULL) AS has_live_agent,
                longest.date        AS longest_date,
                longest.worked      AS longest_worked,
                longest.kind        AS longest_kind,
                open.date           AS open_date,
                open.started_at     AS open_started_at
         FROM users u
         -- The finished day in the window with the most seconds worked. One
         -- day per person and not all of them: the rule fires on the worst,
         -- and a manager does not need eleven rows to be told about a week of
         -- eleven-hour days. The next one surfaces once this is answered.
         LEFT JOIN LATERAL (
             SELECT w.date,
                    w.kind,
                    EXTRACT(EPOCH FROM (w.ended_at - w.started_at))::bigint
                        - COALESCE((SELECT sum(EXTRACT(EPOCH FROM (p.ended_at - p.started_at)))::bigint
                                    FROM pauses p WHERE p.workday_id = w.id AND p.ended_at IS NOT NULL), 0) AS worked
             FROM workdays w
             WHERE w.user_id = u.id AND w.date BETWEEN $1 AND $2 AND w.ended_at IS NOT NULL
             ORDER BY worked DESC
             LIMIT 1
         ) AS longest ON true
         -- The open day, if there is one. Ordered so that if a schema ever let
         -- a person have two, this picks the one that has been open longest -
         -- the one worth saying something about.
         LEFT JOIN LATERAL (
             SELECT w.date, w.started_at
             FROM workdays w
             WHERE w.user_id = u.id AND w.ended_at IS NULL
             ORDER BY w.started_at
             LIMIT 1
         ) AS open ON true
         WHERE u.active",
    )
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Observation::from).collect())
}

/// One person as the sweep's query returns them.
#[derive(Debug, sqlx::FromRow)]
struct ObservationRow {
    user_id: Uuid,
    work_rate: Decimal,
    last_seen_at: Option<DateTime<Utc>>,
    has_live_agent: bool,
    longest_date: Option<NaiveDate>,
    longest_worked: Option<i64>,
    longest_kind: Option<WorkdayKind>,
    open_date: Option<NaiveDate>,
    open_started_at: Option<DateTime<Utc>>,
}

impl From<ObservationRow> for Observation {
    fn from(row: ObservationRow) -> Self {
        Self {
            user_id: row.user_id,
            last_seen_at: row.last_seen_at,
            has_live_agent: row.has_live_agent,
            // All three or none: they come from one `LEFT JOIN LATERAL`, so a
            // partial tuple would mean the query changed shape underneath this.
            longest_finished_day: match (row.longest_date, row.longest_worked, row.longest_kind) {
                (Some(date), Some(worked), Some(kind)) => Some((date, worked.max(0), kind)),
                _ => None,
            },
            open_day: row.open_date.zip(row.open_started_at),
            work_rate: row.work_rate,
        }
    }
}

// The API ---------------------------------------------------------------------

/// One alert, as the feed answers it.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Alert {
    pub id: Uuid,
    pub user_id: Uuid,
    pub display_name: String,
    pub department: Option<String>,
    pub rule: AlertRule,
    pub state: AlertState,
    pub fired_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub acknowledged_at: Option<DateTime<Utc>>,
    /// Who answered it, by name, so the feed can say so without a second
    /// request. Null on one the server resolved by itself - which is how "it
    /// went away" is told from "somebody decided it was fine".
    pub acknowledged_by: Option<String>,
    pub observed_seconds: i64,
    pub against_seconds: Option<i64>,
    pub subject_date: Option<NaiveDate>,
}

/// The feed, and what the reader can say about it.
#[derive(Debug, Serialize)]
pub struct Feed {
    pub alerts: Vec<Alert>,
    /// How many of them are open. The screen's badge, and not `alerts.len()`:
    /// a feed showing the answered ones too would otherwise count them.
    pub open: i64,
    /// People the sweep is watching. `0 of 12` is a different message from
    /// "nothing wrong", and a screen that cannot tell them apart shows the
    /// reassuring one - the same rule the signals band follows.
    pub people: i64,
    /// The thresholds these were raised at, so the screen can say what "too
    /// long" meant without a second request, and an administrator can see the
    /// figure they are about to change.
    pub thresholds: Thresholds,
}

/// What a caller may narrow the feed to.
#[derive(Debug, Deserialize)]
pub struct FeedQuery {
    /// `open` (the default), `all`, or one state by name. Answered rather than
    /// always returning everything: a manager opening the dashboard wants what
    /// is unattended, and the history is a deliberate second click.
    #[serde(default)]
    pub state: Option<String>,
}

/// Answers the alerts about everyone the reader may see.
pub async fn feed(State(state): State<AppState>, user: CurrentUser, Query(query): Query<FeedQuery>) -> Result<impl IntoResponse, ApiError> {
    require_manager_or_admin(&user)?;

    let wanted = query.state.as_deref().unwrap_or("open");
    let filter = match wanted {
        "open" => "AND al.state = 'open'",
        "all" => "",
        "acknowledged" => "AND al.state = 'acknowledged'",
        "resolved" => "AND al.state = 'resolved'",
        other => {
            return Err(ApiError::bad_request(format!(
                "unknown state `{other}`: expected open, acknowledged, resolved or all"
            )));
        }
    };

    // `AssertSqlSafe` because the only interpolations are `VISIBLE_USERS`, a
    // constant in `admin`, and `filter`, matched from a closed set just above;
    // everything from the request is still bound.
    let alerts: Vec<Alert> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT al.id, al.user_id, u.display_name, d.name AS department,
                al.rule, al.state, al.fired_at, al.resolved_at, al.acknowledged_at,
                ack.display_name AS acknowledged_by,
                al.observed_seconds, al.against_seconds, al.subject_date
         FROM alerts al
         JOIN users u ON u.id = al.user_id
         LEFT JOIN departments d ON d.id = u.department_id
         LEFT JOIN users ack ON ack.id = al.acknowledged_by
         WHERE {VISIBLE_USERS} {filter}
         ORDER BY al.fired_at DESC
         LIMIT 200"
    )))
    .bind(user.role == UserRole::Admin)
    .bind(user.user_id)
    .fetch_all(&state.pool)
    .await?;

    let open: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM alerts al JOIN users u ON u.id = al.user_id WHERE {VISIBLE_USERS} AND al.state = 'open'"
    )))
    .bind(user.role == UserRole::Admin)
    .bind(user.user_id)
    .fetch_one(&state.pool)
    .await?;

    let people: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM users u WHERE {VISIBLE_USERS} AND u.active")))
        .bind(user.role == UserRole::Admin)
        .bind(user.user_id)
        .fetch_one(&state.pool)
        .await?;

    // Worst first within the same moment, newest first across moments: a feed
    // read top-down should open with the silence that arrived this morning.
    let mut alerts = alerts;
    alerts.sort_by_key(|alert| (alert.rule.severity(), std::cmp::Reverse(alert.fired_at)));

    let thresholds = Thresholds::load(&state.pool).await?;

    Ok(Json(Feed {
        alerts,
        open,
        people,
        thresholds,
    }))
}

/// Answers an alert: a person looked, and it needs no action.
///
/// Not a delete. The row stays, with who answered it and when, because "this
/// was raised and a human decided it was fine" is the only evidence that the
/// thresholds are set somewhere sensible - and a table whose rows disappear
/// when they are handled can never be asked how it is doing.
pub async fn acknowledge(State(state): State<AppState>, user: CurrentUser, Path(id): Path<Uuid>) -> Result<impl IntoResponse, ApiError> {
    require_manager_or_admin(&user)?;

    // The visibility rule decides this too. Without it a manager could answer
    // an alert about somebody in another department - harmless in itself, and
    // it would tell them that person exists, which ADR 0009 says it must not.
    let owned: Option<(Uuid, AlertRule)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT al.user_id, al.rule FROM alerts al JOIN users u ON u.id = al.user_id WHERE al.id = $3 AND {VISIBLE_USERS}"
    )))
    .bind(user.role == UserRole::Admin)
    .bind(user.user_id)
    .bind(id)
    .fetch_optional(&state.pool)
    .await?;

    // The same 404 for "not yours" as for "no such alert", for the reason the
    // drill-down gives one: a manager probing ids must not be able to tell an
    // employee in another department from one who does not exist.
    let Some((subject, rule)) = owned else {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "no such alert"));
    };

    let updated = sqlx::query(
        "UPDATE alerts SET state = 'acknowledged', acknowledged_at = now(), acknowledged_by = $2
         WHERE id = $1 AND state = 'open'",
    )
    .bind(id)
    .bind(user.user_id)
    .execute(&state.pool)
    .await?;

    if updated.rows_affected() == 0 {
        // Already answered, or resolved itself between the page loading and
        // the click. Said plainly rather than silently succeeding: the screen
        // is about to show a state the reader did not choose.
        return Err(ApiError::new(StatusCode::CONFLICT, "that alert is no longer open"));
    }

    // The target is the person the alert is about, not the alert's own id:
    // the question anybody brings to this log is "what happened to this
    // employee", and an id nobody can resolve afterwards answers nothing.
    audit::Entry::new(audit::action::ALERT_ACKNOWLEDGED)
        .by(user.user_id)
        .by_email(&user.email)
        .on(subject)
        .with(serde_json::json!({ "alert_id": id, "rule": rule }))
        .record(&state.pool)
        .await;

    Ok((StatusCode::OK, Json(serde_json::json!({ "id": id, "state": AlertState::Acknowledged }))))
}

/// The thresholds being changed.
#[derive(Debug, Deserialize)]
pub struct ThresholdsInput {
    pub alert_silence_hours: i32,
    pub alert_overwork_factor: Decimal,
    pub alert_open_day_hours: i32,
}

/// Sets what this installation is willing to be interrupted about.
///
/// All three at once rather than one route each: they are read together on
/// every sweep and shown together on one form, and three routes would let a
/// screen save two of them.
pub async fn put_thresholds(State(state): State<AppState>, user: CurrentUser, Json(input): Json<ThresholdsInput>) -> Result<impl IntoResponse, ApiError> {
    user.require_admin()?;

    // Checked here as well as in the database. The constraint is what makes it
    // true; this is what makes the refusal a sentence rather than a Postgres
    // error code reaching the screen.
    if input.alert_silence_hours <= 0 || input.alert_silence_hours > 720 {
        return Err(ApiError::bad_request("silence is measured in whole hours, from 1 to 720"));
    }
    if input.alert_overwork_factor <= Decimal::ONE || input.alert_overwork_factor > Decimal::from(5) {
        return Err(ApiError::bad_request("overwork is a share of the norm greater than 1 and at most 5"));
    }
    if input.alert_open_day_hours <= 0 || input.alert_open_day_hours > 168 {
        return Err(ApiError::bad_request("an open day is measured in whole hours, from 1 to 168"));
    }

    let previous = Thresholds::load(&state.pool).await?;

    sqlx::query("UPDATE settings SET alert_silence_hours = $1, alert_overwork_factor = $2, alert_open_day_hours = $3 WHERE singleton")
        .bind(input.alert_silence_hours)
        .bind(input.alert_overwork_factor)
        .bind(input.alert_open_day_hours)
        .execute(&state.pool)
        .await?;

    audit::Entry::new(audit::action::ALERT_THRESHOLDS_CHANGED)
        .by(user.user_id)
        .by_email(&user.email)
        .with(serde_json::json!({
            "from": previous,
            "to": {
                "alert_silence_hours": input.alert_silence_hours,
                "alert_overwork_factor": input.alert_overwork_factor,
                "alert_open_day_hours": input.alert_open_day_hours,
            }
        }))
        .record(&state.pool)
        .await;

    let thresholds = Thresholds::load(&state.pool).await?;
    Ok((StatusCode::OK, Json(thresholds)))
}

// The background sweep --------------------------------------------------------

/// How often the sweep runs.
///
/// Five minutes. The conditions are measured in hours, so a finer interval
/// would buy nothing a manager could act on and would put a query over every
/// account on a loop. Coarser, and "a day left open" arrives late enough that
/// somebody has already started working inside the wrong day.
const SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// Runs the sweep on a timer, for as long as the server runs.
///
/// A background task rather than work done when the feed is opened. The whole
/// point of an alert is that it exists before anybody looks: an alert computed
/// on read has no `fired_at` worth the name - it would say the condition began
/// the moment somebody first opened the page - and could never be delivered
/// anywhere, which is what v0.23 is for.
///
/// A sweep that fails is logged and the loop continues. A database blip must
/// not leave a server running with its alerts permanently frozen, and the next
/// sweep reconciles from scratch anyway - there is no state carried between
/// them to be corrupted by a skipped one.
pub fn run_sweeps(pool: PgPool) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
        // The first tick is immediate: a server that has just started should
        // not take five minutes to notice the agent that died while it was
        // down. `Delay` so a burst of missed ticks after a long pause does not
        // run the sweep several times in a row to catch up.
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match sweep(&pool, Utc::now()).await {
                Ok(swept) if swept.raised > 0 || swept.resolved > 0 => {
                    tracing::info!(raised = swept.raised, resolved = swept.resolved, open = swept.unchanged, "swept the alerts");
                }
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "an alert sweep failed; the next one will reconcile from scratch"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::{CalendarDay, CalendarDayKind};

    fn thresholds() -> Thresholds {
        Thresholds {
            alert_silence_hours: 12,
            alert_overwork_factor: Decimal::new(15, 1),
            alert_open_day_hours: 16,
        }
    }

    fn now() -> DateTime<Utc> {
        "2026-09-18T12:00:00Z".parse().expect("a fixed moment for the arithmetic")
    }

    /// Somebody with nothing wrong: an agent that reported an hour ago, an
    /// ordinary day, no open day.
    fn quiet_observation() -> Observation {
        Observation {
            user_id: Uuid::nil(),
            last_seen_at: Some(now() - TimeDelta::hours(1)),
            has_live_agent: true,
            longest_finished_day: Some(("2026-09-17".parse().unwrap(), 8 * SECONDS_PER_HOUR, WorkdayKind::Work)),
            open_day: None,
            work_rate: Decimal::ONE,
        }
    }

    fn eight_hours() -> Decimal {
        Decimal::from(8)
    }

    #[test]
    fn an_ordinary_person_raises_nothing() {
        let found = findings(&quiet_observation(), &Calendar::empty(), eight_hours(), &thresholds(), now());
        assert!(found.is_empty(), "nothing here is worth interrupting anybody about: {found:?}");
    }

    #[test]
    fn silence_past_the_threshold_is_an_alert_and_short_silence_is_not() {
        let mut observation = quiet_observation();

        // Eleven hours is a night. Twelve is the threshold.
        observation.last_seen_at = Some(now() - TimeDelta::hours(11));
        assert!(
            !findings(&observation, &Calendar::empty(), eight_hours(), &thresholds(), now())
                .iter()
                .any(|f| f.rule == AlertRule::NoAgentData),
            "a night's silence is not an alert",
        );

        observation.last_seen_at = Some(now() - TimeDelta::hours(13));
        let found = findings(&observation, &Calendar::empty(), eight_hours(), &thresholds(), now());
        let silence = found
            .iter()
            .find(|f| f.rule == AlertRule::NoAgentData)
            .expect("thirteen hours of silence is an alert");
        assert_eq!(silence.observed_seconds, 13 * SECONDS_PER_HOUR);
        assert_eq!(
            silence.against_seconds,
            Some(12 * SECONDS_PER_HOUR),
            "the alert carries what it was measured against"
        );
        assert_eq!(silence.subject_date, None, "silence is about a person, not about a day");
    }

    #[test]
    fn somebody_with_no_agent_is_not_silent() {
        // The failure this guards is an installation that has not finished
        // handing out tokens alerting about every account on its first day.
        // The dashboard already says "no agents" in words, which is a more
        // useful sentence than "quiet for 400 hours".
        let mut observation = quiet_observation();
        observation.has_live_agent = false;
        observation.last_seen_at = Some(now() - TimeDelta::days(30));

        assert!(
            !findings(&observation, &Calendar::empty(), eight_hours(), &thresholds(), now())
                .iter()
                .any(|f| f.rule == AlertRule::NoAgentData),
            "nothing was ever asked of this person",
        );
    }

    #[test]
    fn overwork_is_measured_against_that_persons_own_norm() {
        let mut observation = quiet_observation();
        let date: NaiveDate = "2026-09-17".parse().unwrap();

        // Full rate: the bar is twelve hours (8 x 1.5). Eleven is a long day
        // and not an alert.
        observation.longest_finished_day = Some((date, 11 * SECONDS_PER_HOUR, WorkdayKind::Work));
        assert!(
            !findings(&observation, &Calendar::empty(), eight_hours(), &thresholds(), now())
                .iter()
                .any(|f| f.rule == AlertRule::Overwork),
            "eleven hours against an eight-hour norm is under the factor",
        );

        observation.longest_finished_day = Some((date, 12 * SECONDS_PER_HOUR, WorkdayKind::Work));
        let found = findings(&observation, &Calendar::empty(), eight_hours(), &thresholds(), now());
        let overwork = found
            .iter()
            .find(|f| f.rule == AlertRule::Overwork)
            .expect("twelve hours is half again the norm");
        assert_eq!(overwork.observed_seconds, 12 * SECONDS_PER_HOUR);
        assert_eq!(
            overwork.against_seconds,
            Some(8 * SECONDS_PER_HOUR),
            "the norm travels with the alert, not a percentage"
        );
        assert_eq!(overwork.subject_date, Some(date));

        // Half time: the same eight-hour day is now double the norm, and the
        // bar is six. This is the whole reason the threshold is a share and
        // not a number of hours - an installation-wide "over ten" would never
        // mention a part-timer working twice their day.
        observation.work_rate = Decimal::new(5, 1);
        observation.longest_finished_day = Some((date, 8 * SECONDS_PER_HOUR, WorkdayKind::Work));
        let found = findings(&observation, &Calendar::empty(), eight_hours(), &thresholds(), now());
        let overwork = found
            .iter()
            .find(|f| f.rule == AlertRule::Overwork)
            .expect("a full day at half rate is double the norm");
        assert_eq!(overwork.against_seconds, Some(4 * SECONDS_PER_HOUR));
    }

    #[test]
    fn a_short_day_before_a_holiday_lowers_the_bar() {
        // The point of reading the norm from the calendar rather than from
        // `standard_hours` alone: on a short day the norm is seven, so the
        // bar is 10.5 rather than 12, and ten and a half hours on the eve of
        // a holiday is exactly the case worth noticing.
        let eve: NaiveDate = "2026-09-17".parse().unwrap();
        let calendar = Calendar::from_days(vec![CalendarDay {
            date: eve,
            kind: CalendarDayKind::ShortDay,
            note: None,
        }]);

        let mut observation = quiet_observation();
        observation.longest_finished_day = Some((eve, 11 * SECONDS_PER_HOUR, WorkdayKind::Work));

        let found = findings(&observation, &calendar, eight_hours(), &thresholds(), now());
        let overwork = found
            .iter()
            .find(|f| f.rule == AlertRule::Overwork)
            .expect("eleven hours against a seven-hour norm is overwork");
        assert_eq!(overwork.against_seconds, Some(7 * SECONDS_PER_HOUR));

        // And with no calendar the same day is an ordinary long one.
        let found = findings(&observation, &Calendar::empty(), eight_hours(), &thresholds(), now());
        assert!(
            !found.iter().any(|f| f.rule == AlertRule::Overwork),
            "the same eleven hours on a full-norm day is under the factor",
        );
    }

    #[test]
    fn a_day_the_calendar_does_not_ask_for_raises_no_overwork() {
        // A Saturday has a norm of zero, and no multiple of zero is anything.
        // Working one is worth noticing and this is not the rule that notices
        // it: there is no norm here to be a factor of, so a threshold would be
        // a number invented after all.
        let saturday: NaiveDate = "2026-09-19".parse().unwrap();
        assert_eq!(saturday.format("%a").to_string(), "Sat", "the fixture has to actually be a Saturday");

        let mut observation = quiet_observation();
        observation.longest_finished_day = Some((saturday, 14 * SECONDS_PER_HOUR, WorkdayKind::Work));

        assert!(
            !findings(&observation, &Calendar::empty(), eight_hours(), &thresholds(), now())
                .iter()
                .any(|f| f.rule == AlertRule::Overwork),
        );
    }

    #[test]
    fn a_day_of_leave_raises_no_overwork() {
        // Its norm is zero, so any work at all would exceed it by an infinite
        // share - and "you worked on your holiday" is between the employee and
        // their own screen, not something to raise with a manager.
        let mut observation = quiet_observation();
        for kind in [WorkdayKind::Vacation, WorkdayKind::Sick, WorkdayKind::DayOff] {
            observation.longest_finished_day = Some(("2026-09-17".parse().unwrap(), 12 * SECONDS_PER_HOUR, kind));
            assert!(
                !findings(&observation, &Calendar::empty(), eight_hours(), &thresholds(), now())
                    .iter()
                    .any(|f| f.rule == AlertRule::Overwork),
                "{kind:?} owes no norm",
            );
        }
    }

    #[test]
    fn a_day_open_too_long_is_an_alert() {
        let mut observation = quiet_observation();
        let date: NaiveDate = "2026-09-18".parse().unwrap();

        // Fifteen hours is a very long day somebody may still be inside.
        observation.open_day = Some((date, now() - TimeDelta::hours(15)));
        assert!(
            !findings(&observation, &Calendar::empty(), eight_hours(), &thresholds(), now())
                .iter()
                .any(|f| f.rule == AlertRule::DayNotClosed),
        );

        observation.open_day = Some((date, now() - TimeDelta::hours(17)));
        let found = findings(&observation, &Calendar::empty(), eight_hours(), &thresholds(), now());
        let open = found
            .iter()
            .find(|f| f.rule == AlertRule::DayNotClosed)
            .expect("seventeen hours open is past any day");
        assert_eq!(open.observed_seconds, 17 * SECONDS_PER_HOUR);
        assert_eq!(open.against_seconds, Some(16 * SECONDS_PER_HOUR));
        assert_eq!(open.subject_date, Some(date));
    }

    #[test]
    fn an_open_day_is_judged_by_the_clock_and_not_by_a_norm() {
        // Half time does not make a day that has been open for seventeen hours
        // any less open. The rule deliberately does not read `work_rate`, and
        // this is what would fail if somebody "made it consistent" with
        // overwork.
        let mut observation = quiet_observation();
        observation.work_rate = Decimal::new(5, 1);
        observation.open_day = Some(("2026-09-18".parse().unwrap(), now() - TimeDelta::hours(17)));

        assert!(
            findings(&observation, &Calendar::empty(), eight_hours(), &thresholds(), now())
                .iter()
                .any(|f| f.rule == AlertRule::DayNotClosed),
        );
    }

    #[test]
    fn the_thresholds_are_what_moves_the_line() {
        // A change an operator makes has to actually change the answer - the
        // failure this guards is a threshold read from settings and then
        // ignored in favour of a constant.
        let mut observation = quiet_observation();
        observation.last_seen_at = Some(now() - TimeDelta::hours(5));

        let strict = Thresholds {
            alert_silence_hours: 4,
            ..thresholds()
        };
        assert!(
            findings(&observation, &Calendar::empty(), eight_hours(), &strict, now())
                .iter()
                .any(|f| f.rule == AlertRule::NoAgentData),
            "five hours of silence is an alert where the threshold is four",
        );
        assert!(
            !findings(&observation, &Calendar::empty(), eight_hours(), &thresholds(), now())
                .iter()
                .any(|f| f.rule == AlertRule::NoAgentData),
            "and is not where the threshold is twelve",
        );
    }

    #[test]
    fn several_rules_can_be_true_at_once() {
        // They are independent facts about one person, and a feed that showed
        // only the worst would hide the open day behind the silence.
        let observation = Observation {
            user_id: Uuid::nil(),
            last_seen_at: Some(now() - TimeDelta::hours(20)),
            has_live_agent: true,
            longest_finished_day: Some(("2026-09-17".parse().unwrap(), 13 * SECONDS_PER_HOUR, WorkdayKind::Work)),
            open_day: Some(("2026-09-18".parse().unwrap(), now() - TimeDelta::hours(20))),
            work_rate: Decimal::ONE,
        };

        let found = findings(&observation, &Calendar::empty(), eight_hours(), &thresholds(), now());
        assert_eq!(found.len(), 3, "silence, overwork and an open day are three separate things: {found:?}");
    }

    #[test]
    fn silence_outranks_the_others_in_the_feed() {
        // An agent that stopped reporting makes every other number about that
        // person untrustworthy, including the two below it.
        let mut rules = [AlertRule::Overwork, AlertRule::NoAgentData, AlertRule::DayNotClosed];
        rules.sort_by_key(|rule| rule.severity());
        assert_eq!(rules, [AlertRule::NoAgentData, AlertRule::DayNotClosed, AlertRule::Overwork]);
    }
}
