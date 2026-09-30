//! Notes on a day: a manager's word on one of an employee's days.
//!
//! "Your day off on Friday is approved." "Thanks for staying for the release."
//! The few sentences a manager has to say about somebody's time travel today
//! through a chat nobody can find again, detached from the day they are about.
//! Here they are written on the day itself, and the person is told (ADR 0021).
//!
//! Not a chat, on purpose. A note goes one way - from whoever may see the day
//! to the person whose day it is - and has no replies: the product measures
//! time, and a thread under every Tuesday would make it a messenger that also
//! measures time. What a note needs is what an alert needs: to be said once,
//! to reach the person on their machine, and to stay next to what it is about.
//!
//! Three rules, each settled before the code:
//!
//! * **On a date, not on a workday.** The note most worth writing is about a
//!   day that has not happened yet - leave approved ahead of time - and that
//!   day has no workday row. A note is keyed like the person's own days, by
//!   who and which local date, and lands on the same line of the week whether
//!   or not kasl ever reports that day.
//! * **Written once.** No edit: the person may have read it already, on their
//!   machine or in the inbox, and words that change under them are words they
//!   cannot rely on. A correction is a second note.
//! * **Withdrawn, not deleted.** A note on the wrong person's day has to be
//!   taken back, and taking it back takes the words out - the row stays so the
//!   notice that announced it can say "withdrawn", the text does not. It is
//!   kept in one place only, so there is no second copy to forget.
//!
//! Who may write is who may see the day: the visibility rule the team screens
//! already apply ([`crate::admin::VISIBLE_USERS`]), asked of the database
//! rather than restated. Who may withdraw is whoever wrote it, or an
//! administrator.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, PgExecutor};
use uuid::Uuid;

use crate::{admin::require_manager_or_admin, app::AppState, audit, error::ApiError, login::CurrentUser, me::Range, model::UserRole, notifications, team};

/// The longest note, in characters. A note, not a letter: what does not fit
/// in a toast on somebody's laptop belongs in a conversation.
pub const MAX_CHARS: usize = 1000;

/// How far ahead a note may be dated. Leave is approved weeks ahead, sometimes
/// months; a note two years out is a mistyped year, and it would sit on a line
/// of the week nobody opens until it is long forgotten.
pub const MAX_DAYS_AHEAD: i64 = 366;

/// A note as every screen reads it.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Note {
    pub id: Uuid,
    /// The person's local date it is written on.
    pub date: NaiveDate,
    pub text: String,
    /// Who wrote it, so a screen can offer its author the way to withdraw it.
    /// Absent once that account is gone from the installation.
    pub author_id: Option<Uuid>,
    /// Their name as it is now. The notice that announced the note keeps the
    /// name it was written under; this is the day's own view of it.
    pub author: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// The standing notes on one person's days in a range, oldest first.
///
/// Withdrawn notes are not among them: a withdrawn note has no words left, and
/// the day is not the place to say one used to be there - the inbox is, where
/// the notice about it is shown as withdrawn. Callers check who may read what
/// before they get here, as they do for the days beside these.
pub async fn for_range(executor: impl PgExecutor<'_>, user_id: Uuid, range: &Range) -> Result<Vec<Note>, sqlx::Error> {
    sqlx::query_as(
        "SELECT n.id, n.date, n.text, n.author_id, a.display_name AS author, n.created_at
         FROM day_notes n
         LEFT JOIN users a ON a.id = n.author_id
         WHERE n.user_id = $1 AND n.date BETWEEN $2 AND $3 AND n.withdrawn_at IS NULL
         ORDER BY n.date, n.created_at, n.id",
    )
    .bind(user_id)
    .bind(range.from)
    .bind(range.to)
    .fetch_all(executor)
    .await
}

/// A note being written.
#[derive(Debug, Deserialize)]
pub struct NewNote {
    pub date: NaiveDate,
    pub text: String,
}

/// Checks a note before anything is written, and answers the text to store.
///
/// Separate from the handler so the rules can be read - and tested - without a
/// database behind them. `today` is the server's: a note is refused for being
/// too far ahead of the day the server is in, not of the author's.
pub fn validate(new: &NewNote, today: NaiveDate) -> Result<String, ApiError> {
    // Trimmed, so a note of spaces is refused as empty rather than stored as
    // one, and so the limit counts what the person will read.
    let text = new.text.trim();
    if text.is_empty() {
        return Err(ApiError::bad_request("a note needs some text"));
    }
    let length = text.chars().count();
    if length > MAX_CHARS {
        return Err(ApiError::bad_request(format!("a note is at most {MAX_CHARS} characters, this one is {length}")));
    }
    if new.date > today + Duration::days(MAX_DAYS_AHEAD) {
        return Err(ApiError::bad_request(format!(
            "a note is dated at most {MAX_DAYS_AHEAD} days ahead; {} is further",
            new.date
        )));
    }
    Ok(text.to_string())
}

/// `POST /api/v1/users/{id}/notes`: writes a note on one of that person's
/// days, and tells them.
///
/// The note and the notice are one transaction - the rule every notice
/// follows (ADR 0020): a note written and never told is the chat message this
/// feature exists to replace, and a notice about a note that did not commit
/// would announce nothing.
pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(target): Path<Uuid>,
    Json(new): Json<NewNote>,
) -> Result<impl IntoResponse, ApiError> {
    require_manager_or_admin(&user)?;

    if !team::may_read(&state.pool, &user, target).await? {
        // The drill-down's 404, for the drill-down's reason: a manager probing
        // ids must not be able to tell a person in another department from
        // one who does not exist (ADR 0009).
        return Err(ApiError::new(StatusCode::NOT_FOUND, "no such user"));
    }
    if target == user.user_id {
        // A manager is visible to themselves, so the rule above lets this
        // through. A note on your own day would be told to its own author.
        return Err(ApiError::bad_request("a note is written on somebody else's day"));
    }

    let text = validate(&new, Utc::now().date_naive())?;

    let (active, subject_email): (bool, String) = sqlx::query_as("SELECT active, email FROM users WHERE id = $1")
        .bind(target)
        .fetch_one(&state.pool)
        .await?;
    if !active {
        // Nobody left to read it: a deactivated account signs in nowhere and
        // its agents are refused.
        return Err(ApiError::new(StatusCode::CONFLICT, "that account is deactivated"));
    }

    let mut tx = state.pool.begin().await?;
    let note = write(&mut tx, target, new.date, user.user_id, &text).await?;
    tx.commit().await?;

    // The words are not in the entry. The audit log outlives a withdrawal by
    // design (ADR 0010), and a note taken back because it was about the wrong
    // person must not survive in the one table nobody can delete from.
    audit::Entry::new(audit::action::NOTE_ADDED)
        .by(user.user_id)
        .by_email(&user.email)
        .on(target)
        .labelled(subject_email)
        .with(serde_json::json!({ "note_id": note.id, "date": note.date }))
        .record(&state.pool)
        .await;

    Ok((StatusCode::CREATED, Json(note)))
}

/// Writes a note and the notice that tells its person, in the caller's
/// transaction.
///
/// The one way a note is made - the route above and the demo's seed alike, so
/// the demo shows the rows a real note leaves (the same reason its days go
/// through `import::write_day`). Checks nothing: whoever calls it has decided
/// the author may write here and the text is a note.
pub async fn write(conn: &mut PgConnection, user_id: Uuid, date: NaiveDate, author_id: Uuid, text: &str) -> Result<Note, sqlx::Error> {
    let note: Note = sqlx::query_as(
        "WITH written AS (
             INSERT INTO day_notes (user_id, date, author_id, text) VALUES ($1, $2, $3, $4)
             RETURNING id, date, text, author_id, created_at
         )
         SELECT w.id, w.date, w.text, w.author_id, a.display_name AS author, w.created_at
         FROM written w LEFT JOIN users a ON a.id = w.author_id",
    )
    .bind(user_id)
    .bind(date)
    .bind(author_id)
    .bind(text)
    .fetch_one(&mut *conn)
    .await?;

    notifications::note_added(
        conn,
        user_id,
        &notifications::NoteFact {
            id: note.id,
            date: note.date,
            author: note.author.clone().unwrap_or_default(),
            text: None,
        },
    )
    .await?;
    Ok(note)
}

/// What a withdrawal leaves.
#[derive(Debug, Serialize)]
pub struct Withdrawn {
    pub id: Uuid,
    pub withdrawn_at: DateTime<Utc>,
}

/// `DELETE /api/v1/notes/{id}`: takes a note back.
///
/// The row stays and the words go. The notice that announced it is then
/// withdrawn with it - not toasted on a machine that has not shown it yet,
/// kept in the inbox as withdrawn - because it reads the words from this row
/// and whether it is still true from this row's `withdrawn_at` (ADR 0021).
/// Nothing new is told: "your manager took a note back" is not something to
/// act on, and a channel that says everything teaches people to ignore it.
pub async fn withdraw(State(state): State<AppState>, user: CurrentUser, Path(id): Path<Uuid>) -> Result<impl IntoResponse, ApiError> {
    require_manager_or_admin(&user)?;

    let found: Option<(Uuid, NaiveDate, Option<Uuid>, bool, String)> = sqlx::query_as(
        "SELECT n.user_id, n.date, n.author_id, n.withdrawn_at IS NOT NULL, u.email
         FROM day_notes n JOIN users u ON u.id = n.user_id
         WHERE n.id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?;

    let Some((subject, date, author, withdrawn, subject_email)) = found else {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "no such note"));
    };

    let wrote_it = author == Some(user.user_id);
    if !wrote_it && user.role != UserRole::Admin {
        // Somebody who can see the day sees the note on it, and is told why
        // they cannot take it back. Anybody else gets the answer a note that
        // does not exist gets: whose days carry notes is not theirs to learn.
        return Err(if team::may_read(&state.pool, &user, subject).await? {
            ApiError::new(StatusCode::FORBIDDEN, "only whoever wrote a note, or an administrator, can withdraw it")
        } else {
            ApiError::new(StatusCode::NOT_FOUND, "no such note")
        });
    }
    if withdrawn {
        return Err(ApiError::new(StatusCode::CONFLICT, "that note is already withdrawn"));
    }

    // `withdrawn_at IS NULL` in the statement as well as above: two clicks
    // racing each other withdraw it once, and the second is told so.
    let withdrawn_at: Option<DateTime<Utc>> =
        sqlx::query_scalar("UPDATE day_notes SET text = NULL, withdrawn_at = now() WHERE id = $1 AND withdrawn_at IS NULL RETURNING withdrawn_at")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?;
    let Some(withdrawn_at) = withdrawn_at else {
        return Err(ApiError::new(StatusCode::CONFLICT, "that note is already withdrawn"));
    };

    audit::Entry::new(audit::action::NOTE_WITHDRAWN)
        .by(user.user_id)
        .by_email(&user.email)
        .on(subject)
        .labelled(subject_email)
        .with(serde_json::json!({ "note_id": id, "date": date, "by_author": wrote_it }))
        .record(&state.pool)
        .await;

    Ok(Json(Withdrawn { id, withdrawn_at }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(date: &str, text: &str) -> NewNote {
        NewNote {
            date: date.parse().expect("a test date"),
            text: text.to_string(),
        }
    }

    fn today() -> NaiveDate {
        "2026-09-30".parse().unwrap()
    }

    #[test]
    fn a_note_is_stored_as_it_will_be_read() {
        // The spaces around it are the text box's, not the author's.
        assert_eq!(validate(&note("2026-10-02", "  Approved.\n"), today()).unwrap(), "Approved.");
    }

    #[test]
    fn a_note_of_nothing_is_refused() {
        let error = validate(&note("2026-10-02", " \n\t "), today()).unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn the_length_is_counted_in_characters() {
        // Characters, not bytes: a Cyrillic note of the full length is twice
        // as many bytes and still one note.
        let full = "ж".repeat(MAX_CHARS);
        assert!(
            validate(&note("2026-10-02", &full), today()).is_ok(),
            "exactly {MAX_CHARS} characters is a note"
        );

        let over = "ж".repeat(MAX_CHARS + 1);
        let error = validate(&note("2026-10-02", &over), today()).unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
        assert!(error.to_string().contains("1001"), "the message names the length: {error}");
    }

    #[test]
    fn a_note_may_be_dated_ahead_up_to_a_year() {
        // The edge on both sides: leave approved a year out is a note, a year
        // and a day is a mistyped year.
        let edge = today() + Duration::days(MAX_DAYS_AHEAD);
        assert!(validate(&note(&edge.to_string(), "Approved."), today()).is_ok());

        let past_it = edge + Duration::days(1);
        let error = validate(&note(&past_it.to_string(), "Approved."), today()).unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn a_note_on_a_past_day_is_fine() {
        // History is imported years back, and a word on an old day is as much
        // a note as one on today.
        assert!(validate(&note("2023-01-09", "This was the migration week."), today()).is_ok());
    }
}
