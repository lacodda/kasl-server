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
//! * **The server does not own the scale.** It answers seconds and the
//!   busiest day; how dark a square is belongs to the screen.
//!
//! Since 0.28 it is a projection of the month's timesheet (ADR 0024) rather
//! than a query of its own: the same people - including somebody who worked
//! in the month and has left since - and the same figures, read from
//! `workday_figures`, so the shape and the record of one month cannot
//! disagree about anybody.

use axum::{
    Json,
    extract::{Query, State},
    response::IntoResponse,
};
use chrono::NaiveDate;
use serde::Serialize;
use uuid::Uuid;

use crate::{
    admin::require_manager_or_admin,
    app::AppState,
    calendar::WorkdayKind,
    error::ApiError,
    login::CurrentUser,
    timesheet::{self, MonthQuery},
};

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
    /// The kind of day the agent recorded. A day of sick leave is a cell -
    /// something was recorded - but not a day worked, and a reader shading it
    /// by its hours would draw it as the shortest working day of the month.
    pub kind: WorkdayKind,
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
    /// The longest finished day of work in this row, in seconds, or `null`
    /// when the row has none.
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
    let timesheet = timesheet::build(&state.pool, user.reader(), &query.month, None).await?;

    let rows: Vec<Row> = timesheet.rows.iter().map(row).collect();
    let busiest_seconds = rows.iter().filter_map(|row| row.busiest_seconds).max();

    Ok(Json(Heatmap {
        month: timesheet.month,
        from: timesheet.from,
        to: timesheet.to,
        rows,
        busiest_seconds,
    }))
}

/// One person's month as the heatmap answers it: the dates something was
/// recorded on, and nothing else.
fn row(person: &timesheet::Row) -> Row {
    let days: Vec<Cell> = person
        .days
        .iter()
        .filter_map(|cell| {
            Some(Cell {
                date: cell.date,
                worked_seconds: cell.worked_seconds,
                open: cell.state == timesheet::CellState::Open,
                // Only a recorded date has a kind; the rest are not cells here.
                kind: cell.kind?,
            })
        })
        .collect();

    let busiest_seconds = days
        .iter()
        .filter(|day| day.kind == WorkdayKind::Work)
        .filter_map(|day| day.worked_seconds)
        .max();

    Row {
        user_id: person.member.id,
        display_name: person.member.display_name.clone(),
        department: person.member.department.clone(),
        days,
        busiest_seconds,
        // The summary's figure: the month's worked seconds, to the second.
        worked_seconds: person.member.worked_seconds,
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;

    use super::*;
    use crate::{
        team::Member,
        timesheet::{CellState, Counts},
    };

    fn date(text: &str) -> NaiveDate {
        text.parse().expect("a test date")
    }

    fn member(worked_seconds: i64) -> Member {
        Member {
            id: Uuid::nil(),
            display_name: "Ann".into(),
            email: "ann@example.test".into(),
            department: None,
            department_id: None,
            active: true,
            days_recorded: 0,
            worked_seconds,
            paused_seconds: 0,
            last_day: None,
            day_open: false,
            last_seen_at: None,
            agents: 0,
            work_rate: Decimal::ONE,
            norm_seconds: 0,
            due_seconds: 0,
            days_away: 0,
        }
    }

    fn cell(text: &str, state: CellState, kind: Option<WorkdayKind>, worked: Option<i64>) -> timesheet::Cell {
        timesheet::Cell {
            date: date(text),
            state,
            kind,
            worked_seconds: worked,
            norm_seconds: 0,
            filed: None,
            report: None,
        }
    }

    fn person(days: Vec<timesheet::Cell>, worked_seconds: i64) -> timesheet::Row {
        timesheet::Row {
            member: member(worked_seconds),
            counts: Counts::default(),
            days,
        }
    }

    #[test]
    fn a_person_with_nothing_recorded_is_still_a_row() {
        // The empty row is the case the screen exists for: an employee whose
        // agent never reported must be visible, not quietly dropped - and the
        // dates owed and unrecorded are absent cells, never zeroes.
        let row = row(&person(
            vec![
                cell("2026-09-01", CellState::Unrecorded, None, None),
                cell("2026-09-05", CellState::Free, None, None),
                cell("2026-09-30", CellState::Ahead, None, None),
            ],
            0,
        ));

        assert!(row.days.is_empty(), "no days, rather than days with nothing in them");
        assert_eq!(row.worked_seconds, 0);
        assert_eq!(row.busiest_seconds, None, "no finished day means no busiest one");
    }

    #[test]
    fn an_open_day_is_a_cell_without_a_total() {
        let row = row(&person(
            vec![
                cell("2026-09-01", CellState::Worked, Some(WorkdayKind::Work), Some(28_800)),
                cell("2026-09-02", CellState::Open, Some(WorkdayKind::Work), None),
            ],
            28_800,
        ));

        assert_eq!(row.days.len(), 2);
        assert_eq!(row.days[1].worked_seconds, None);
        assert!(row.days[1].open);
        assert!(!row.days[0].open);
        assert_eq!(row.busiest_seconds, Some(28_800));
    }

    #[test]
    fn a_day_away_is_a_cell_with_its_kind_and_never_the_busiest() {
        // Before 0.28 a sick day arrived as a day with no hours and no kind,
        // and a screen shading by hours drew it as a very short day worked.
        let row = row(&person(
            vec![
                cell("2026-09-01", CellState::Worked, Some(WorkdayKind::Work), Some(3_600)),
                cell("2026-09-02", CellState::Away, Some(WorkdayKind::Sick), Some(36_000)),
            ],
            3_600,
        ));

        assert_eq!(row.days[1].kind, WorkdayKind::Sick);
        assert_eq!(row.busiest_seconds, Some(3_600), "the busiest day is a day of work");
    }

    #[test]
    fn the_busiest_day_is_the_largest_not_the_latest() {
        // Guards against `busiest = seconds` overwriting on every day, which a
        // month of ascending dates would hide.
        let row = row(&person(
            vec![
                cell("2026-09-01", CellState::Worked, Some(WorkdayKind::Work), Some(36_000)),
                cell("2026-09-02", CellState::Worked, Some(WorkdayKind::Work), Some(3_600)),
            ],
            39_600,
        ));

        assert_eq!(row.busiest_seconds, Some(36_000));
        assert_eq!(row.worked_seconds, 39_600, "the summary's total, to the second");
    }
}
