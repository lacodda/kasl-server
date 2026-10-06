//! Reports, and their approval: the person says a day is finished, and - where
//! the installation asks for it - a manager approves it (ADR 0022).
//!
//! `kasl report --send` has always been how an employee closes a day: it ends
//! the workday and sends the day's report to whoever the company has it go to.
//! A report here is the same act, received by the team server: the person puts
//! their name to what a day came to. Approval is the second half, and it is
//! opt-in - an installation that never asked for it must not grow a queue of
//! days waiting for somebody.
//!
//! Three rules, each settled before the code:
//!
//! * **A report is an event, never edited.** The person said, at that moment,
//!   that the day came to these figures. Corrected in kasl and reported again,
//!   it is a second report; what a day's report says now is its newest one.
//! * **The day stays kasl's.** A day re-sent after it was approved is stored
//!   as sent, like any other (ADR 0004) - an agent is never refused because a
//!   manager answered. The approval was of the figures it names, and once the
//!   day no longer comes to them it stops covering the day: the report reads
//!   as `changed`, and the person reports it again.
//! * **The status is derived, never stored.** Whether a report still describes
//!   its day is a comparison between the figures in the report and what the
//!   day comes to now (`workday_figures`), made on every read. A stored status
//!   would be a second copy of that comparison, and every path that writes a
//!   day would have to remember to update it.
//!
//! Who may approve is who may see the day - the visibility rule the team
//! screens already apply ([`crate::admin::VISIBLE_USERS`]) - and never the
//! person whose day it is.

use std::collections::BTreeMap;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, PgExecutor};
use uuid::Uuid;

use crate::{
    admin::{VISIBLE_USERS, require_manager_or_admin},
    app::AppState,
    audit,
    auth::AuthenticatedAgent,
    calendar::WorkdayKind,
    error::ApiError,
    login::CurrentUser,
    me::Range,
    model::UserRole,
    notifications::{self, ApprovedFact, ReportedDay, ReturnedFact},
};

/// The longest reason a day is returned with, in characters. A reason, not a
/// letter: what does not fit in a toast belongs in a conversation.
pub const MAX_REASON_CHARS: usize = 1000;

/// How many reports one approval may carry. A team's month, with room to
/// spare; a request naming more is not somebody clicking "approve all".
pub const MAX_APPROVE: usize = 500;

/// How many waiting reports the queue answers at once. The oldest first, which
/// are the ones to answer; the count says how many more there are.
const QUEUE_PAGE: i64 = 200;

/// What a manager decided. Mirrors the `report_review` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "report_review", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum Review {
    Approved,
    Returned,
}

/// Where a day's report stands, read from its newest report and the day as it
/// is now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Reported, and nobody has answered. Where the installation approves
    /// days, it waits for a manager; where it does not, that is all it is.
    Submitted,
    /// A manager approved the figures, and the day still comes to them.
    Approved,
    /// A manager sent it back, with a reason. The person reports it again.
    Returned,
    /// The day no longer comes to what was reported - kasl sent it again with
    /// other figures, before an answer or after one. The person reports it
    /// again.
    Changed,
}

/// The status of a report, from what a manager said and whether the day has
/// moved since.
///
/// A returned report stays returned whatever the day does next: the person
/// was asked to look at it, and that is still the thing to do. Otherwise a day
/// that moved outranks an answer - an approval covers the figures it names,
/// not the date.
pub fn status(review: Option<Review>, changed: bool) -> Status {
    match (review, changed) {
        (Some(Review::Returned), _) => Status::Returned,
        (_, true) => Status::Changed,
        (Some(Review::Approved), false) => Status::Approved,
        (None, false) => Status::Submitted,
    }
}

/// A report as every screen reads it.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub id: Uuid,
    pub user_id: Uuid,
    pub date: NaiveDate,
    pub status: Status,
    pub submitted_at: DateTime<Utc>,
    /// The figures as reported. Beside the day's own on the screen, so it can
    /// say "approved at 7.5 h, the day now comes to 7.8 h" rather than only
    /// that something moved.
    pub kind: WorkdayKind,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub worked_seconds: i64,
    pub reviewed_at: Option<DateTime<Utc>>,
    /// Who answered, so a screen can name them. Absent once that account is
    /// gone from the installation.
    pub reviewer_id: Option<Uuid>,
    pub reviewer: Option<String>,
    /// Why it was returned. Only on a returned report.
    pub reason: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct Row {
    id: Uuid,
    user_id: Uuid,
    date: NaiveDate,
    kind: WorkdayKind,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    worked_seconds: i64,
    submitted_at: DateTime<Utc>,
    review: Option<Review>,
    reviewed_at: Option<DateTime<Utc>>,
    reviewer_id: Option<Uuid>,
    reviewer: Option<String>,
    reason: Option<String>,
    changed: bool,
}

impl From<Row> for Report {
    fn from(row: Row) -> Self {
        Self {
            id: row.id,
            user_id: row.user_id,
            date: row.date,
            status: status(row.review, row.changed),
            submitted_at: row.submitted_at,
            kind: row.kind,
            started_at: row.started_at,
            ended_at: row.ended_at,
            worked_seconds: row.worked_seconds,
            reviewed_at: row.reviewed_at,
            reviewer_id: row.reviewer_id,
            reviewer: row.reviewer,
            reason: row.reason,
        }
    }
}

/// Whether the day no longer comes to what a report says, as a column.
///
/// Expects `r` (the report) and `f` (`workday_figures`, left-joined on the
/// person and the date). A day that is gone has moved as surely as one whose
/// hours did, and so has one that kasl reopened. `IS DISTINCT FROM` because the
/// day's end is null while it is open, and a plain `<>` against null is null -
/// which a `WHERE` drops without a word.
const CHANGED: &str =
    "(f.id IS NULL OR (f.kind, f.started_at, f.ended_at, f.worked_seconds) IS DISTINCT FROM (r.kind, r.started_at, r.ended_at, r.worked_seconds))";

/// A report's columns as [`Row`] reads them. Expects `r`, `f` and `rv` (the
/// reviewer, left-joined).
const COLUMNS: &str = "r.id, r.user_id, r.date, r.kind, r.started_at, r.ended_at, r.worked_seconds, r.submitted_at,
     r.review, r.reviewed_at, r.reviewed_by AS reviewer_id, rv.display_name AS reviewer, r.reason";

/// The joins [`COLUMNS`] and [`CHANGED`] expect, from `reports r`.
const JOINS: &str = "FROM reports r
     LEFT JOIN workday_figures f ON f.user_id = r.user_id AND f.date = r.date
     LEFT JOIN users rv ON rv.id = r.reviewed_by";

/// The order of one day's reports, newest first. `id` breaks a tie no clock
/// produces, so the order is a total one rather than the database's choice.
const NEWEST_FIRST: &str = "r.submitted_at DESC, r.id DESC";

/// The newest report of each of a person's days in a range, oldest day first.
///
/// What a day's report says now. An older report of the same day is history -
/// a correction made since, or an answer given to figures that are gone.
pub async fn for_range(executor: impl PgExecutor<'_>, user_id: Uuid, range: &Range) -> Result<Vec<Report>, sqlx::Error> {
    // `AssertSqlSafe` on constants in this module; the values are bound.
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT DISTINCT ON (r.date) {COLUMNS}, {CHANGED} AS changed
         {JOINS}
         WHERE r.user_id = $1 AND r.date BETWEEN $2 AND $3
         ORDER BY r.date, {NEWEST_FIRST}"
    )))
    .bind(user_id)
    .bind(range.from)
    .bind(range.to)
    .fetch_all(executor)
    .await?;
    Ok(rows.into_iter().map(Report::from).collect())
}

/// The newest report of one day, if it has any.
async fn newest(conn: &mut PgConnection, user_id: Uuid, date: NaiveDate) -> Result<Option<Report>, sqlx::Error> {
    let reports = for_range(&mut *conn, user_id, &Range { from: date, to: date }).await?;
    Ok(reports.into_iter().next())
}

/// Reports by id, as they read now.
async fn by_ids(conn: &mut PgConnection, ids: &[Uuid]) -> Result<Vec<Report>, sqlx::Error> {
    // `AssertSqlSafe` on constants in this module; the ids are bound.
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS}, {CHANGED} AS changed {JOINS} WHERE r.id = ANY($1) ORDER BY r.date, r.user_id"
    )))
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(Report::from).collect())
}

/// Whether this installation asks for days to be approved.
pub async fn approval_enabled(executor: impl PgExecutor<'_>) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT day_approval FROM settings WHERE singleton")
        .fetch_one(executor)
        .await
}

/// Refuses an answer where nobody asked for one.
async fn require_approval(executor: impl PgExecutor<'_>) -> Result<(), ApiError> {
    if approval_enabled(executor).await? {
        Ok(())
    } else {
        Err(ApiError::new(StatusCode::CONFLICT, "this server does not ask for days to be approved"))
    }
}

// Reporting --------------------------------------------------------------------------

/// The day being reported.
#[derive(Debug, Deserialize)]
pub struct ReportRequest {
    pub date: NaiveDate,
}

/// What reporting a day did.
#[derive(Debug)]
pub struct Submitted {
    pub report: Report,
    /// Whether this request wrote it, or a report already standing covers
    /// these very figures.
    pub created: bool,
}

/// Reports one of a person's days, in the caller's transaction.
///
/// The one way a report is made - the person's own route, the agent's, and
/// the demo's seed - so the demo shows the rows a real one leaves.
///
/// Sending the same day twice is safe: while the newest report covers the
/// figures the day comes to now, and nobody sent it back, it is answered again
/// rather than written again. An agent that lost the answer to the network
/// retries without filling a manager's queue with copies.
pub async fn submit(conn: &mut PgConnection, user_id: Uuid, date: NaiveDate) -> Result<Submitted, ApiError> {
    // The day is locked first, the way an upload locks it (`ingest`), so the
    // figures read below are the ones the report is written with - an upload
    // of the same day waits for this, or this waits for it.
    let workday: Option<Uuid> = sqlx::query_scalar("SELECT id FROM workdays WHERE user_id = $1 AND date = $2 FOR UPDATE")
        .bind(user_id)
        .bind(date)
        .fetch_optional(&mut *conn)
        .await?;
    let Some(workday) = workday else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("there is no day on {date} to report: kasl has not sent it"),
        ));
    };

    let (kind, started_at, ended_at, worked_seconds): (WorkdayKind, DateTime<Utc>, Option<DateTime<Utc>>, Option<i64>) =
        sqlx::query_as("SELECT kind, started_at, ended_at, worked_seconds FROM workday_figures WHERE id = $1")
            .bind(workday)
            .fetch_one(&mut *conn)
            .await?;
    let (Some(ended_at), Some(worked_seconds)) = (ended_at, worked_seconds) else {
        // A report is the person saying the day is finished, and this one is
        // not - by kasl's own account. Its hours are not a figure yet.
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!("the day of {date} is still open; finish it in kasl before reporting it"),
        ));
    };

    if let Some(standing) = newest(conn, user_id, date).await?
        && matches!(standing.status, Status::Submitted | Status::Approved)
    {
        return Ok(Submitted {
            report: standing,
            created: false,
        });
    }

    let id: Uuid =
        sqlx::query_scalar("INSERT INTO reports (user_id, date, kind, started_at, ended_at, worked_seconds) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id")
            .bind(user_id)
            .bind(date)
            .bind(kind)
            .bind(started_at)
            .bind(ended_at)
            .bind(worked_seconds)
            .fetch_one(&mut *conn)
            .await?;

    let report = by_ids(conn, &[id]).await?.into_iter().next().expect("the report was just written");
    Ok(Submitted { report, created: true })
}

fn answer(submitted: Submitted) -> impl IntoResponse {
    let status = if submitted.created { StatusCode::CREATED } else { StatusCode::OK };
    (status, Json(submitted.report))
}

/// `POST /api/v1/me/reports`: the person reports one of their days.
///
/// Whether or not the installation approves days: a report is the person
/// saying the day is finished, and that is true either way. With approval on
/// it is also a question to their manager.
pub async fn submit_own(State(state): State<AppState>, user: CurrentUser, Json(request): Json<ReportRequest>) -> Result<impl IntoResponse, ApiError> {
    let mut tx = state.pool.begin().await?;
    let submitted = submit(&mut tx, user.user_id, request.date).await?;
    tx.commit().await?;
    Ok(answer(submitted))
}

/// `POST /api/v1/agent/reports`: the same, from kasl - which is where the
/// person closes their day (`kasl report --send`), and so where reporting it
/// belongs first.
pub async fn submit_from_agent(
    State(state): State<AppState>,
    agent: AuthenticatedAgent,
    Json(request): Json<ReportRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let mut tx = state.pool.begin().await?;
    let submitted = submit(&mut tx, agent.user_id, request.date).await?;
    tx.commit().await?;
    tracing::info!(user_id = %agent.user_id, agent_id = %agent.agent_id, date = %request.date, created = submitted.created, "reported a day");
    Ok(answer(submitted))
}

// Answering ----------------------------------------------------------------------------

/// Who is answering, as the rules need them.
#[derive(Debug, Clone, Copy)]
pub struct Reviewer {
    pub id: Uuid,
    pub is_admin: bool,
}

impl From<&CurrentUser> for Reviewer {
    fn from(user: &CurrentUser) -> Self {
        Self {
            id: user.user_id,
            is_admin: user.role == UserRole::Admin,
        }
    }
}

/// A report as an answer needs it: where it stands, whether it is the day's
/// newest, and whether the reviewer may see it at all.
#[derive(Debug, sqlx::FromRow)]
struct Candidate {
    id: Uuid,
    user_id: Uuid,
    date: NaiveDate,
    kind: WorkdayKind,
    worked_seconds: i64,
    review: Option<Review>,
    changed: bool,
    newest: bool,
    visible: bool,
    email: String,
}

/// Locks the reports named and reads what deciding about them needs.
async fn candidates(conn: &mut PgConnection, reviewer: Reviewer, ids: &[Uuid]) -> Result<Vec<Candidate>, sqlx::Error> {
    // Locked before they are read, so two managers answering the same report
    // at once answer it one after the other, and the second sees the first's
    // answer.
    sqlx::query("SELECT id FROM reports WHERE id = ANY($1) ORDER BY id FOR UPDATE")
        .bind(ids)
        .execute(&mut *conn)
        .await?;

    // `AssertSqlSafe` on `CHANGED` and `VISIBLE_USERS`, constants; everything
    // else is bound. The rule is read as a column here rather than as a
    // filter, and for a person in no department it is null rather than false -
    // which a `WHERE` drops without a word and a column hands over as is.
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT r.id, r.user_id, r.date, r.kind, r.worked_seconds, r.review,
                {CHANGED} AS changed,
                NOT EXISTS (
                    SELECT 1 FROM reports later
                    WHERE later.user_id = r.user_id AND later.date = r.date
                      AND (later.submitted_at, later.id) > (r.submitted_at, r.id)
                ) AS newest,
                coalesce({VISIBLE_USERS}, false) AS visible,
                u.email
         FROM reports r
         JOIN users u ON u.id = r.user_id
         LEFT JOIN workday_figures f ON f.user_id = r.user_id AND f.date = r.date
         WHERE r.id = ANY($3)"
    )))
    .bind(reviewer.is_admin)
    .bind(reviewer.id)
    .bind(ids)
    .fetch_all(&mut *conn)
    .await
}

/// Why a report cannot be approved, or `None` when it can. Separate from the
/// handler so the rules can be read - and tested - without a database.
fn refuse_approval(reviewer: Reviewer, candidate: &Candidate) -> Option<&'static str> {
    if !candidate.visible {
        // The answer a report that does not exist gets: whose days carry
        // reports is not the reviewer's to learn (ADR 0009).
        return Some("no such report");
    }
    if candidate.user_id == reviewer.id {
        return Some("nobody approves their own day");
    }
    if !candidate.newest {
        return Some("the day was reported again since; answer the newest report");
    }
    if candidate.review == Some(Review::Returned) {
        return Some("this report was returned; it is approved once the day is reported again");
    }
    if candidate.changed {
        return Some("the day changed after it was reported; it is approved once the day is reported again");
    }
    None
}

/// What an approval did.
#[derive(Debug, Default, Serialize)]
pub struct Approval {
    /// The reports approved, including any that already were.
    pub approved: Vec<Report>,
    /// The ones that could not be, each with the reason.
    pub refused: Vec<Refusal>,
}

#[derive(Debug, Serialize)]
pub struct Refusal {
    pub id: Uuid,
    pub error: String,
}

/// A report newly approved, for the audit log.
#[derive(Debug)]
pub struct Approved {
    pub id: Uuid,
    pub user_id: Uuid,
    pub date: NaiveDate,
    pub email: String,
}

/// Approves reports in the caller's transaction, and tells each person once.
///
/// One notice per person rather than one per day: approving a team's week is
/// one act, and five toasts saying the same thing teach a person to stop
/// reading them (ADR 0020). Approving a report that already stands approved
/// answers it as approved and says nothing again.
pub async fn approve_in(conn: &mut PgConnection, reviewer: Reviewer, ids: &[Uuid]) -> Result<(Approval, Vec<Approved>), ApiError> {
    let found = candidates(conn, reviewer, ids).await?;

    let mut outcome = Approval::default();
    let mut newly: Vec<&Candidate> = Vec::new();
    let mut approved_ids: Vec<Uuid> = Vec::new();

    for id in ids {
        let Some(candidate) = found.iter().find(|candidate| candidate.id == *id) else {
            outcome.refused.push(Refusal {
                id: *id,
                error: "no such report".to_string(),
            });
            continue;
        };
        if let Some(reason) = refuse_approval(reviewer, candidate) {
            outcome.refused.push(Refusal {
                id: *id,
                error: reason.to_string(),
            });
            continue;
        }
        approved_ids.push(*id);
        if candidate.review.is_none() {
            newly.push(candidate);
        }
    }

    if !newly.is_empty() {
        let ids: Vec<Uuid> = newly.iter().map(|candidate| candidate.id).collect();
        sqlx::query("UPDATE reports SET review = 'approved', reviewed_by = $1, reviewed_at = now() WHERE id = ANY($2)")
            .bind(reviewer.id)
            .bind(&ids)
            .execute(&mut *conn)
            .await?;

        let name = reviewer_name(conn, reviewer.id).await?;
        // A `BTreeMap` so the people are told in a fixed order and each list
        // of days reads oldest first.
        let mut by_person: BTreeMap<Uuid, Vec<ReportedDay>> = BTreeMap::new();
        for candidate in &newly {
            by_person.entry(candidate.user_id).or_default().push(ReportedDay {
                date: candidate.date,
                kind: candidate.kind,
                worked_seconds: candidate.worked_seconds,
            });
        }
        for (user_id, mut days) in by_person {
            days.sort_by_key(|day| day.date);
            notifications::reports_approved(conn, user_id, &ApprovedFact { reviewer: name.clone(), days }).await?;
        }
    }

    outcome.approved = by_ids(conn, &approved_ids).await?;
    let newly = newly
        .into_iter()
        .map(|candidate| Approved {
            id: candidate.id,
            user_id: candidate.user_id,
            date: candidate.date,
            email: candidate.email.clone(),
        })
        .collect();
    Ok((outcome, newly))
}

async fn reviewer_name(conn: &mut PgConnection, id: Uuid) -> Result<String, sqlx::Error> {
    sqlx::query_scalar("SELECT display_name FROM users WHERE id = $1")
        .bind(id)
        .fetch_one(conn)
        .await
}

/// The reports being approved.
#[derive(Debug, Deserialize)]
pub struct ApproveRequest {
    pub ids: Vec<Uuid>,
}

/// `POST /api/v1/reports/approve`: approves the reports named.
///
/// A list from the start, because "approve everything waiting" is what a
/// manager does on a Friday, and a route that took one report would make that
/// a request per day. Each is decided on its own: one that cannot be approved
/// is listed with the reason and does not stop the rest, the way a day in an
/// upload batch is (ADR 0005).
pub async fn approve(State(state): State<AppState>, user: CurrentUser, Json(request): Json<ApproveRequest>) -> Result<impl IntoResponse, ApiError> {
    require_manager_or_admin(&user)?;
    let mut ids = request.ids;
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Err(ApiError::bad_request("name at least one report to approve"));
    }
    if ids.len() > MAX_APPROVE {
        return Err(ApiError::bad_request(format!(
            "one approval carries at most {MAX_APPROVE} reports, this one carries {}",
            ids.len()
        )));
    }

    let mut tx = state.pool.begin().await?;
    require_approval(&mut *tx).await?;
    let (outcome, newly) = approve_in(&mut tx, Reviewer::from(&user), &ids).await?;
    tx.commit().await?;

    for approved in newly {
        audit::Entry::new(audit::action::REPORT_APPROVED)
            .by(user.user_id)
            .by_email(&user.email)
            .on(approved.user_id)
            .labelled(approved.email)
            .with(serde_json::json!({ "report_id": approved.id, "date": approved.date }))
            .record(&state.pool)
            .await;
    }

    Ok(Json(outcome))
}

/// Checks a reason before anything is written, and answers the text to store.
pub fn validate_reason(reason: &str) -> Result<String, ApiError> {
    // Trimmed, so a reason of spaces is refused as empty, and the limit counts
    // what the person will read.
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(ApiError::bad_request("say why the day is returned: the person has to know what to look at"));
    }
    let length = reason.chars().count();
    if length > MAX_REASON_CHARS {
        return Err(ApiError::bad_request(format!(
            "a reason is at most {MAX_REASON_CHARS} characters, this one is {length}"
        )));
    }
    Ok(reason.to_string())
}

/// What returning a report needs that approving does not.
#[derive(Debug, Deserialize)]
pub struct ReturnRequest {
    pub reason: String,
}

/// Sends a report back, in the caller's transaction, and tells the person.
///
/// A report waiting for an answer can be returned, and so can one already
/// approved - a manager who notices on Monday what they approved on Friday has
/// to be able to say so. One that was returned already cannot: the person has
/// been asked, and the next word is theirs.
pub async fn return_in(conn: &mut PgConnection, reviewer: Reviewer, id: Uuid, reason: &str) -> Result<(Report, String), ApiError> {
    let found = candidates(conn, reviewer, &[id]).await?;
    let Some(candidate) = found.into_iter().next().filter(|candidate| candidate.visible) else {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "no such report"));
    };
    if candidate.user_id == reviewer.id {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "nobody returns their own day"));
    }
    if !candidate.newest {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "the day was reported again since; answer the newest report",
        ));
    }
    if candidate.review == Some(Review::Returned) {
        return Err(ApiError::new(StatusCode::CONFLICT, "this report was returned already"));
    }

    sqlx::query("UPDATE reports SET review = 'returned', reason = $2, reviewed_by = $3, reviewed_at = now() WHERE id = $1")
        .bind(id)
        .bind(reason)
        .bind(reviewer.id)
        .execute(&mut *conn)
        .await?;

    let name = reviewer_name(conn, reviewer.id).await?;
    notifications::report_returned(
        conn,
        candidate.user_id,
        &ReturnedFact {
            id,
            date: candidate.date,
            reviewer: name,
            reason: None,
        },
    )
    .await?;

    let report = by_ids(conn, &[id]).await?.into_iter().next().expect("the report was just answered");
    Ok((report, candidate.email))
}

/// `POST /api/v1/reports/{id}/return`: sends a report back with a reason.
pub async fn send_back(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(request): Json<ReturnRequest>,
) -> Result<impl IntoResponse, ApiError> {
    require_manager_or_admin(&user)?;
    let reason = validate_reason(&request.reason)?;

    let mut tx = state.pool.begin().await?;
    require_approval(&mut *tx).await?;
    let (report, email) = return_in(&mut tx, Reviewer::from(&user), id, &reason).await?;
    tx.commit().await?;

    // Not the words. A reason is said about one person to that person, and
    // the audit log is the one table read by every administrator and kept
    // forever (ADR 0010); the report holds it, where the person reads it.
    audit::Entry::new(audit::action::REPORT_RETURNED)
        .by(user.user_id)
        .by_email(&user.email)
        .on(report.user_id)
        .labelled(email)
        .with(serde_json::json!({ "report_id": report.id, "date": report.date }))
        .record(&state.pool)
        .await;

    Ok(Json(report))
}

// The queue ------------------------------------------------------------------------------

/// A report waiting for an answer, with whose it is.
#[derive(Debug, Serialize)]
pub struct WaitingReport {
    #[serde(flatten)]
    pub report: Report,
    pub display_name: String,
    pub department: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct WaitingRow {
    #[sqlx(flatten)]
    report: Row,
    display_name: String,
    department: Option<String>,
    total: i64,
}

/// What waits for the reader.
#[derive(Debug, Serialize)]
pub struct Queue {
    /// Whether this installation approves days at all. Off, the queue is
    /// empty because nothing is asked, which a screen says differently from
    /// "you have answered everything".
    pub day_approval: bool,
    /// Oldest day first: the longest-waiting answer is the one to give next.
    pub reports: Vec<WaitingReport>,
    /// How many wait in all, of which `reports` is the first page.
    pub waiting: i64,
}

/// `GET /api/v1/team/reports`: the reports waiting for this reader's answer.
///
/// The newest report of each day of everybody the reader may see, that
/// nobody has answered and that still describes its day - and never the
/// reader's own. A report whose day moved since is not waiting for a manager;
/// it is waiting for its person to report the day again.
pub async fn queue(State(state): State<AppState>, user: CurrentUser) -> Result<impl IntoResponse, ApiError> {
    require_manager_or_admin(&user)?;

    if !approval_enabled(&state.pool).await? {
        return Ok(Json(Queue {
            day_approval: false,
            reports: Vec::new(),
            waiting: 0,
        }));
    }

    // The newest report per day is picked first and filtered after: picking
    // among the unanswered ones would surface an older report of a day whose
    // newest was already returned.
    //
    // `AssertSqlSafe` on constants in this module and `VISIBLE_USERS`; the
    // values are bound.
    let rows: Vec<WaitingRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT n.*, u.display_name, d.name AS department, count(*) OVER () AS total
         FROM (
             SELECT DISTINCT ON (r.user_id, r.date) {COLUMNS}, {CHANGED} AS changed
             {JOINS}
             JOIN users u ON u.id = r.user_id
             WHERE u.active AND r.user_id <> $2 AND {VISIBLE_USERS}
             ORDER BY r.user_id, r.date, {NEWEST_FIRST}
         ) n
         JOIN users u ON u.id = n.user_id
         LEFT JOIN departments d ON d.id = u.department_id
         WHERE n.review IS NULL AND NOT n.changed
         ORDER BY n.date, u.display_name, u.email
         LIMIT $3"
    )))
    .bind(user.role == UserRole::Admin)
    .bind(user.user_id)
    .bind(QUEUE_PAGE)
    .fetch_all(&state.pool)
    .await?;

    let waiting = rows.first().map_or(0, |row| row.total);
    Ok(Json(Queue {
        day_approval: true,
        reports: rows
            .into_iter()
            .map(|row| WaitingReport {
                report: row.report.into(),
                display_name: row.display_name,
                department: row.department,
            })
            .collect(),
        waiting,
    }))
}

// The setting -----------------------------------------------------------------------------

/// Whether this installation approves days.
#[derive(Debug, Serialize, Deserialize)]
pub struct ApprovalSetting {
    pub enabled: bool,
}

/// `GET /api/v1/reports/approval`. Readable by anyone signed in: whether their
/// days are approved is not a secret from the people whose days they are.
pub async fn show_setting(State(state): State<AppState>, _user: CurrentUser) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(ApprovalSetting {
        enabled: approval_enabled(&state.pool).await?,
    }))
}

/// `PUT /api/v1/reports/approval`. Administrators only, and recorded.
///
/// Turning it off answers nothing and erases nothing: approvals given stand
/// as given, and reports waiting stop waiting - they are reports, which is
/// what they were before anybody asked. Turned on again, they wait again.
pub async fn update_setting(State(state): State<AppState>, user: CurrentUser, Json(update): Json<ApprovalSetting>) -> Result<impl IntoResponse, ApiError> {
    user.require_admin()?;

    let previous = approval_enabled(&state.pool).await?;
    sqlx::query("UPDATE settings SET day_approval = $1 WHERE singleton")
        .bind(update.enabled)
        .execute(&state.pool)
        .await?;

    if previous != update.enabled {
        tracing::info!(enabled = update.enabled, by = %user.user_id, "changed whether days are approved");
        audit::Entry::new(audit::action::DAY_APPROVAL_CHANGED)
            .by(user.user_id)
            .by_email(&user.email)
            .with(serde_json::json!({ "from": previous, "to": update.enabled }))
            .record(&state.pool)
            .await;
    }

    Ok(Json(ApprovalSetting { enabled: update.enabled }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_reads_from_its_answer_and_its_day() {
        assert_eq!(status(None, false), Status::Submitted);
        assert_eq!(status(Some(Review::Approved), false), Status::Approved);
        // An approval covers figures, not a date: once the day moves, the
        // report says so rather than "approved".
        assert_eq!(status(Some(Review::Approved), true), Status::Changed);
        assert_eq!(status(None, true), Status::Changed);
        // Returned stays returned: the person was asked to look, and a day
        // that moved since does not answer that.
        assert_eq!(status(Some(Review::Returned), false), Status::Returned);
        assert_eq!(status(Some(Review::Returned), true), Status::Returned);
    }

    fn candidate() -> Candidate {
        Candidate {
            id: Uuid::from_u128(1),
            user_id: Uuid::from_u128(2),
            date: "2026-10-02".parse().unwrap(),
            kind: WorkdayKind::Work,
            worked_seconds: 27_000,
            review: None,
            changed: false,
            newest: true,
            visible: true,
            email: "inside@example.test".to_string(),
        }
    }

    fn manager() -> Reviewer {
        Reviewer {
            id: Uuid::from_u128(3),
            is_admin: false,
        }
    }

    #[test]
    fn a_waiting_report_of_somebody_else_is_approved() {
        assert_eq!(refuse_approval(manager(), &candidate()), None);
        // So is one already approved: the answer is the same, and saying it
        // twice changes nothing.
        let approved = Candidate {
            review: Some(Review::Approved),
            ..candidate()
        };
        assert_eq!(refuse_approval(manager(), &approved), None);
    }

    #[test]
    fn each_refusal_says_why() {
        let cases = [
            (Candidate { visible: false, ..candidate() }, "no such report"),
            (
                Candidate {
                    user_id: manager().id,
                    ..candidate()
                },
                "own day",
            ),
            (Candidate { newest: false, ..candidate() }, "reported again"),
            (
                Candidate {
                    review: Some(Review::Returned),
                    ..candidate()
                },
                "returned",
            ),
            (Candidate { changed: true, ..candidate() }, "changed"),
        ];
        for (case, expected) in cases {
            let reason = refuse_approval(manager(), &case).unwrap_or_else(|| panic!("{case:?} must be refused"));
            assert!(reason.contains(expected), "`{reason}` should mention `{expected}`");
        }
    }

    #[test]
    fn an_invisible_report_is_refused_before_anything_is_said_about_it() {
        // A report on a day the reviewer cannot see does not exist to them,
        // even when it is also their own, or stale: the first refusal would
        // otherwise confirm that the report is there.
        let hidden = Candidate {
            visible: false,
            changed: true,
            newest: false,
            ..candidate()
        };
        assert_eq!(refuse_approval(manager(), &hidden), Some("no such report"));
    }

    #[test]
    fn a_reason_is_stored_as_it_will_be_read() {
        assert_eq!(
            validate_reason("  Friday is missing its lunch break.\n").unwrap(),
            "Friday is missing its lunch break."
        );
        assert_eq!(validate_reason(" \n ").unwrap_err().status(), StatusCode::BAD_REQUEST);

        // Characters, not bytes, at the edge on both sides.
        assert!(validate_reason(&"ж".repeat(MAX_REASON_CHARS)).is_ok());
        let error = validate_reason(&"ж".repeat(MAX_REASON_CHARS + 1)).unwrap_err();
        assert!(error.to_string().contains("1001"), "{error}");
    }

    #[test]
    fn the_wire_names_are_the_contract() {
        // kasl and the web UI branch on these. A rename is a new API version.
        for (status, name) in [
            (Status::Submitted, "submitted"),
            (Status::Approved, "approved"),
            (Status::Returned, "returned"),
            (Status::Changed, "changed"),
        ] {
            assert_eq!(serde_json::to_value(status).unwrap(), serde_json::json!(name));
        }
    }
}
