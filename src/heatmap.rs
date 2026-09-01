//! The month as a shape: `GET /api/v1/team/heatmap`.
//!
//! The dashboard answers a week as totals and a moment as a pulse. Neither
//! shows the pattern a manager actually reads a team by - who works weekends,
//! whose month is ragged, who stopped filing days on the 12th. This endpoint
//! is `kasl sum` widened by one axis: a row per person, a cell per local date.
//!
//! Three rules, settled in ADR 0015 before the code:
//!
//! * **A missing cell is missing data.** Only dates with a workday are
//!   answered. Filling the month with zeroes would make the employee who never
//!   installed kasl look like the one who took the month off, and the first
//!   reading a manager reaches for is the wrong one of the two.
//! * **An open day has no total.** `worked_seconds` is `null` while the day is
//!   running, as `/me/days` answers it. A half-lived day is not a short day.
//! * **The server does not own the scale.** It answers seconds; what counts as
//!   a full day is a norm, and this installation has none until v0.21. A
//!   threshold shipped in the API would be an invention that is hard to take
//!   back.

use axum::{
    Json,
    extract::{Query, State},
    response::IntoResponse,
};
use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    admin::{VISIBLE_USERS, require_manager_or_admin},
    app::AppState,
    error::ApiError,
    login::CurrentUser,
    model::UserRole,
};

/// The month being asked for, as `YYYY-MM`.
///
/// A month rather than a free range: the screen pages by month, the calendar
/// resolves its own length so February is never off by a day, and anyone who
/// wants an arbitrary span already has `/me/days`.
#[derive(Debug, Deserialize)]
pub struct MonthQuery {
    pub month: String,
}

/// One day of one person's month.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Cell {
    /// The employee's own local date, as their agent recorded it (ADR 0003).
    pub date: NaiveDate,
    /// Seconds worked: the day's span less what was paused in it. `null` for a
    /// day still open - it has no total yet, and reporting the hours so far
    /// would draw a full day as a short one.
    pub worked_seconds: Option<i64>,
    /// Whether the day is still running on the agent.
    pub open: bool,
}

/// One person's month.
#[derive(Debug, Serialize)]
pub struct Row {
    pub user_id: Uuid,
    pub display_name: String,
    pub department: Option<String>,
    /// Only the dates with a workday, ascending. An empty vector is a real
    /// answer: this person recorded nothing this month.
    pub days: Vec<Cell>,
    /// The longest finished day in this row, in seconds, or `null` when the
    /// row has no finished day.
    pub busiest_seconds: Option<i64>,
    /// The month's total across finished days.
    pub worked_seconds: i64,
}

/// The team's month.
#[derive(Debug, Serialize)]
pub struct Heatmap {
    /// The month asked for, echoed as `YYYY-MM`.
    pub month: String,
    /// Its first and last dates, so the screen draws the right number of
    /// columns without repeating the calendar arithmetic.
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub rows: Vec<Row>,
    /// The busiest single day anywhere in the answer. The shared ceiling a
    /// screen needs if it wants one scale across the whole grid rather than a
    /// per-row one, which would make a light week look like a heavy one.
    pub busiest_seconds: Option<i64>,
}

/// Answers the team's month.
pub async fn month(State(state): State<AppState>, user: CurrentUser, Query(query): Query<MonthQuery>) -> Result<impl IntoResponse, ApiError> {
    require_manager_or_admin(&user)?;
    let (from, to) = month_bounds(&query.month)?;

    let is_admin = user.role == UserRole::Admin;

    // Grouped in the database rather than folded in Rust: the alternative
    // fetches every workday of the month for the whole team and reassembles it
    // here, which is the same rows over the wire for nothing.
    //
    // `AssertSqlSafe` because the only interpolation is `VISIBLE_USERS`, a
    // constant in `admin`; every value from the request is bound below.
    let cells: Vec<CellRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT u.id AS user_id, u.display_name, d.name AS department,
                w.date, w.ended_at IS NULL AS open,
                CASE WHEN w.ended_at IS NULL THEN NULL
                     ELSE greatest(extract(epoch FROM (w.ended_at - w.started_at))::bigint - paused.seconds, 0)
                END AS worked_seconds
         FROM users u
         LEFT JOIN departments d ON d.id = u.department_id
         LEFT JOIN workdays w ON w.user_id = u.id AND w.date BETWEEN $3 AND $4
         LEFT JOIN LATERAL (
             -- Stored pauses where they exist; the day's own totals where a
             -- narrower policy summarized them away (ADR 0011). One or the
             -- other, never both, so an hour cannot be counted twice.
             SELECT CASE
                 WHEN EXISTS (SELECT 1 FROM pauses p WHERE p.workday_id = w.id)
                 THEN (SELECT coalesce(sum(p.duration_seconds), 0)::bigint FROM pauses p WHERE p.workday_id = w.id)
                 ELSE coalesce(w.paused_seconds, 0)::bigint
             END AS seconds
         ) AS paused ON true
         WHERE u.active AND {VISIBLE_USERS}
         ORDER BY u.display_name, u.email, w.date"
    )))
    .bind(is_admin)
    .bind(user.user_id)
    .bind(from)
    .bind(to)
    .fetch_all(&state.pool)
    .await?;

    let rows = into_rows(cells);
    let busiest_seconds = rows.iter().filter_map(|row| row.busiest_seconds).max();

    Ok(Json(Heatmap {
        month: query.month,
        from,
        to,
        rows,
        busiest_seconds,
    }))
}

/// The first and last date of a `YYYY-MM` month.
///
/// Separate from the handler so the parsing - and its refusals - can be read
/// and tested without a database behind them. The end comes from the calendar
/// rather than from adding 30 days, which is where February goes wrong.
pub fn month_bounds(month: &str) -> Result<(NaiveDate, NaiveDate), ApiError> {
    let shape = || ApiError::bad_request(format!("`month` must be YYYY-MM, got `{month}`"));

    // Parsed as a whole date so `2026-08-15` is refused rather than silently
    // read as August: a caller passing a date means to ask something this
    // endpoint does not answer, and quietly widening it hides their mistake.
    let (year, rest) = month.split_once('-').ok_or_else(shape)?;
    if rest.len() != 2 || year.len() != 4 {
        return Err(shape());
    }
    let first = NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d").map_err(|_| shape())?;

    // The first of the next month, stepped back one day: the only arithmetic
    // that gets December and February right without a table of lengths.
    let next = if first.month() == 12 {
        NaiveDate::from_ymd_opt(first.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(first.year(), first.month() + 1, 1)
    };
    let last = next.and_then(|next| next.pred_opt()).ok_or_else(shape)?;

    Ok((first, last))
}

/// A row of the query: a person, and one of their days where they have any.
///
/// The `LEFT JOIN` means a person with nothing recorded still arrives, with
/// every day column null. That is the row a manager most needs to see, so it
/// is carried through rather than filtered out.
#[derive(Debug, sqlx::FromRow)]
struct CellRow {
    user_id: Uuid,
    display_name: String,
    department: Option<String>,
    date: Option<NaiveDate>,
    open: Option<bool>,
    worked_seconds: Option<i64>,
}

/// Folds the flat rows into one entry per person.
///
/// Relies on the query's `ORDER BY` grouping each person's rows together, so
/// this is a single pass rather than a map keyed by id - and the order the
/// database chose is the order the screen draws.
fn into_rows(cells: Vec<CellRow>) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();

    for cell in cells {
        if rows.last().map(|row| row.user_id) != Some(cell.user_id) {
            rows.push(Row {
                user_id: cell.user_id,
                display_name: cell.display_name,
                department: cell.department,
                days: Vec::new(),
                busiest_seconds: None,
                worked_seconds: 0,
            });
        }

        let row = rows.last_mut().expect("a row was just pushed for this person");

        // No date means the outer join found nothing for this person - the
        // person is the answer, the day is not.
        let Some(date) = cell.date else { continue };

        if let Some(seconds) = cell.worked_seconds {
            row.worked_seconds += seconds;
            row.busiest_seconds = Some(row.busiest_seconds.map_or(seconds, |busiest: i64| busiest.max(seconds)));
        }

        row.days.push(Cell {
            date,
            worked_seconds: cell.worked_seconds,
            // A day that arrived without the flag is treated as finished: the
            // column it comes from is `NOT NULL`, so this cannot happen, and
            // guessing "open" would put a running day on a month long past.
            open: cell.open.unwrap_or(false),
        });
    }

    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(month: &str) -> (NaiveDate, NaiveDate) {
        month_bounds(month).expect("a valid month")
    }

    fn date(text: &str) -> NaiveDate {
        text.parse().expect("a test date")
    }

    #[test]
    fn a_month_ends_where_the_calendar_says() {
        // The three lengths, and the one only a leap year gets right.
        assert_eq!(bounds("2026-01"), (date("2026-01-01"), date("2026-01-31")));
        assert_eq!(bounds("2026-04"), (date("2026-04-01"), date("2026-04-30")));
        assert_eq!(bounds("2026-02"), (date("2026-02-01"), date("2026-02-28")));
        assert_eq!(bounds("2024-02"), (date("2024-02-01"), date("2024-02-29")));
    }

    #[test]
    fn december_rolls_into_the_next_year() {
        // The month after December is not month 13 - arithmetic that forgets
        // this answers an empty grid every January.
        assert_eq!(bounds("2026-12"), (date("2026-12-01"), date("2026-12-31")));
    }

    #[test]
    fn a_month_that_is_not_a_month_is_refused() {
        // `2026-08-15` among them: a caller passing a date is asking for
        // something else, and widening it to the month hides their mistake.
        for bad in ["2026", "2026-13", "2026-00", "August", "2026-08-15", "26-08", ""] {
            let error = month_bounds(bad).unwrap_err();
            assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST, "`{bad}` should be refused");
            assert!(error.to_string().contains("YYYY-MM"), "the message should say the shape: {error}");
        }
    }

    fn cell(user: Uuid, name: &str, date: Option<&str>, worked: Option<i64>, open: bool) -> CellRow {
        CellRow {
            user_id: user,
            display_name: name.to_string(),
            department: None,
            date: date.map(|text| text.parse().expect("a test date")),
            open: date.map(|_| open),
            worked_seconds: worked,
        }
    }

    #[test]
    fn a_person_with_nothing_recorded_is_still_a_row() {
        // The empty row is the case the screen exists for: an employee whose
        // agent never reported must be visible, not quietly dropped.
        let nobody = Uuid::new_v4();
        let rows = into_rows(vec![cell(nobody, "Nobody", None, None, false)]);

        assert_eq!(rows.len(), 1);
        assert!(rows[0].days.is_empty(), "no days, rather than a day with no date");
        assert_eq!(rows[0].worked_seconds, 0);
        assert_eq!(rows[0].busiest_seconds, None, "no finished day means no busiest one");
    }

    #[test]
    fn an_open_day_is_a_cell_without_a_total() {
        // It counts as a day recorded and contributes nothing to the totals: a
        // day still running has no figure, and treating its hours-so-far as
        // one would draw a full day as a short one.
        let person = Uuid::new_v4();
        let rows = into_rows(vec![
            cell(person, "Ann", Some("2026-09-01"), Some(28_800), false),
            cell(person, "Ann", Some("2026-09-02"), None, true),
        ]);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].days.len(), 2);
        assert_eq!(rows[0].days[1].worked_seconds, None);
        assert!(rows[0].days[1].open);
        assert_eq!(rows[0].worked_seconds, 28_800, "the open day adds nothing");
        assert_eq!(rows[0].busiest_seconds, Some(28_800));
    }

    #[test]
    fn each_persons_days_land_on_their_own_row() {
        let ann = Uuid::new_v4();
        let bob = Uuid::new_v4();
        let rows = into_rows(vec![
            cell(ann, "Ann", Some("2026-09-01"), Some(3_600), false),
            cell(ann, "Ann", Some("2026-09-02"), Some(7_200), false),
            cell(bob, "Bob", Some("2026-09-01"), Some(1_800), false),
        ]);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].days.len(), 2);
        assert_eq!(rows[0].worked_seconds, 10_800);
        assert_eq!(rows[0].busiest_seconds, Some(7_200), "the longest day, not the last one");
        assert_eq!(rows[1].days.len(), 1);
        assert_eq!(rows[1].worked_seconds, 1_800);
    }

    #[test]
    fn the_busiest_day_is_the_largest_not_the_latest() {
        // Guards the fold against `busiest = seconds` overwriting on every
        // row, which a month of ascending dates would hide.
        let person = Uuid::new_v4();
        let rows = into_rows(vec![
            cell(person, "Ann", Some("2026-09-01"), Some(36_000), false),
            cell(person, "Ann", Some("2026-09-02"), Some(3_600), false),
        ]);

        assert_eq!(rows[0].busiest_seconds, Some(36_000));
    }
}
