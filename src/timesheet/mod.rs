//! The month as a record: `GET /api/v1/team/timesheet` and
//! `GET /api/v1/me/timesheet` (ADR 0024).
//!
//! A timesheet is what a month came to, person by person and date by date, in
//! the shape a company files it: every date of the month saying what it was -
//! a day worked and its hours, a day away and why, a day owed and never
//! recorded, a day still to come - with the month's totals beside it. It is the
//! team's half of `kasl timesheet`, which files one person's month from their
//! own machine.
//!
//! Three rules, settled in ADR 0024 before the code:
//!
//! * **A row is the summary's row.** The people and their totals come from
//!   [`team::members`], the function behind the team table and the summary
//!   export, so a timesheet cannot disagree with either about anybody's month -
//!   and somebody who worked in it and has left since is on it, because a month
//!   is a record of what happened in it.
//! * **Every date is answered, and the server says what it was.** Which dates
//!   were owed, which have come due and which are still ahead is the norm's
//!   rule (ADR 0017, ADR 0023); a screen that worked it out again would be a
//!   second copy of it. A date that was owed, has come due and holds nothing
//!   is answered as exactly that - never as a zero (ADR 0015).
//! * **A report is on time when it arrives before the next working day
//!   begins.** No setting, and no time zone the server does not have (ADR
//!   0003): the deadline is a moment the agent already reported - when the
//!   person started work again.

mod workbook;

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Query, State},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    admin::require_manager_or_admin,
    app::AppState,
    audit,
    calendar::{self, Calendar, CalendarDayKind, Norm, WorkdayKind},
    error::ApiError,
    export,
    login::CurrentUser,
    me::Range,
    reports::{self, Status},
    team::{self, Member},
};

/// The month being asked for, as `YYYY-MM`.
///
/// A month rather than a free range: a timesheet is filed by the month, the
/// calendar resolves its own length so February is never off by a day, and a
/// free range already has the days export.
#[derive(Debug, Deserialize)]
pub struct MonthQuery {
    pub month: String,
}

/// The first and last date of a `YYYY-MM` month.
///
/// Separate from the handlers so the parsing - and its refusals - can be read
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

// The answer ------------------------------------------------------------------

/// What a date was, for one person.
///
/// Six facts rather than a number, because only one of them is a number. A
/// timesheet that drew "owed and nothing recorded" as a zero would tell a
/// manager somebody worked nothing when nobody said so, and one that drew a
/// holiday the same way would accuse the whole team at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CellState {
    /// A finished day of work, with hours behind it.
    Worked,
    /// A day of work still running on the agent: no total yet.
    Open,
    /// A day the person was on leave, ill or off - the kind says which. It
    /// owes nothing (ADR 0017).
    Away,
    /// Owed, come due, and nothing recorded.
    Unrecorded,
    /// Owed, and not come due yet: a date ahead, or today before its day is
    /// in.
    Ahead,
    /// Owed nothing and nothing recorded: a weekend, a holiday, a share of a
    /// full day of zero.
    Free,
}

/// Whether a day was reported in time.
///
/// The deadline is the start of the person's next working day (ADR 0024): a
/// report is the person saying a day is finished, and the moment they start
/// another one is the moment that should have been said.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Filed {
    /// Reported before the next working day began - or with none begun yet.
    OnTime,
    /// First reported after the next working day began.
    Late,
    /// Not reported, and the next working day has begun; or a date that was
    /// owed, has come due and has no day to report at all.
    Missing,
    /// Not reported yet, and there is still time: the next working day has not
    /// begun.
    Pending,
}

/// One date of one person's month.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Cell {
    pub date: NaiveDate,
    pub state: CellState,
    /// The kind of day the agent recorded. Absent where nothing was.
    pub kind: Option<WorkdayKind>,
    /// Seconds worked, as `workday_figures` gives them. Absent for an open day
    /// - it has no total yet - and where nothing was recorded.
    pub worked_seconds: Option<i64>,
    /// What the date asked of this person: the calendar at their share of a
    /// full day, and nothing on a day they were away. The row's cells add up
    /// to its `norm_seconds`.
    pub norm_seconds: i64,
    /// Whether the day was reported in time. Only where a report is asked
    /// for: a finished day of work, or a date owed and gone by unrecorded.
    pub filed: Option<Filed>,
    /// Where the day's newest report stands (ADR 0022), when it has one.
    pub report: Option<Status>,
}

/// A row's dates counted by what they were.
///
/// Counted here rather than on the screen so the file and the screen say the
/// same numbers: a total is part of the record, not a matter of drawing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub vacation: i64,
    pub sick: i64,
    pub day_off: i64,
    /// Dates owed, come due, and holding nothing.
    pub unrecorded: i64,
    pub on_time: i64,
    pub late: i64,
    pub missing: i64,
    pub pending: i64,
    /// Days whose newest report a manager approved and that still come to it.
    pub approved: i64,
}

impl Counts {
    fn of(cells: &[Cell]) -> Self {
        let mut counts = Self::default();
        for cell in cells {
            match (cell.state, cell.kind) {
                (CellState::Away, Some(WorkdayKind::Vacation)) => counts.vacation += 1,
                (CellState::Away, Some(WorkdayKind::Sick)) => counts.sick += 1,
                (CellState::Away, Some(WorkdayKind::DayOff)) => counts.day_off += 1,
                (CellState::Unrecorded, _) => counts.unrecorded += 1,
                _ => {}
            }
            match cell.filed {
                Some(Filed::OnTime) => counts.on_time += 1,
                Some(Filed::Late) => counts.late += 1,
                Some(Filed::Missing) => counts.missing += 1,
                Some(Filed::Pending) => counts.pending += 1,
                None => {}
            }
            if cell.report == Some(Status::Approved) {
                counts.approved += 1;
            }
        }
        counts
    }
}

/// One person's month: the summary's row, and a cell for every date.
#[derive(Debug, Serialize)]
pub struct Row {
    /// The same row the team table and the summary export answer, field for
    /// field (ADR 0023): the totals a timesheet files are the totals the
    /// screen showed.
    #[serde(flatten)]
    pub member: Member,
    /// Every date of the month, in order.
    pub days: Vec<Cell>,
    pub counts: Counts,
}

/// One date of the month, as the calendar has it - the column a timesheet
/// draws, and its header.
#[derive(Debug, Clone, Serialize)]
pub struct Column {
    pub date: NaiveDate,
    /// What the date is worth at a full day: the calendar's rule (ADR 0017).
    /// The plan row of a timesheet.
    pub hours: Decimal,
    /// What the calendar says about the date, when it says anything.
    pub calendar: Option<CalendarDayKind>,
    /// What the date is called - "New Year's Day" - when the calendar names
    /// it.
    pub note: Option<String>,
    /// Saturday or Sunday, before the calendar has its say. A working weekend
    /// is both a weekend and a full day.
    pub weekend: bool,
}

/// A month of the team, or of one person.
#[derive(Debug, Serialize)]
pub struct Timesheet {
    /// The month asked for, echoed as `YYYY-MM`.
    pub month: String,
    pub from: NaiveDate,
    pub to: NaiveDate,
    /// The server's date, which "come due" is measured by (ADR 0023).
    pub today: NaiveDate,
    /// The installation's full day, in hours.
    pub standard_hours: Decimal,
    /// Whether this installation approves days, so a screen can say what an
    /// unanswered report is waiting for - or that it waits for nobody.
    pub day_approval: bool,
    pub dates: Vec<Column>,
    pub rows: Vec<Row>,
}

// What the database holds about a day -----------------------------------------

/// A recorded day, with the two moments its report is measured by.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct Recorded {
    pub(crate) user_id: Uuid,
    pub(crate) date: NaiveDate,
    pub(crate) kind: WorkdayKind,
    pub(crate) worked_seconds: Option<i64>,
    pub(crate) open: bool,
    /// When the person's next working day began, if one has.
    pub(crate) next_started_at: Option<DateTime<Utc>>,
    /// When the day was first reported, if it has been.
    pub(crate) first_reported_at: Option<DateTime<Utc>>,
}

async fn recorded(pool: &PgPool, ids: &[Uuid], range: &Range) -> Result<Vec<Recorded>, ApiError> {
    // The next working day is the next date with a day of work on it, however
    // far ahead - a Friday's deadline is Monday morning, and after a fortnight
    // away it is the Monday after. The first report rather than the newest:
    // a day reported on time and corrected later was still reported on time.
    // Both are subqueries on the indexes they need: `workdays (user_id, date)`
    // and `reports (user_id, date, submitted_at)`.
    Ok(sqlx::query_as(
        "SELECT f.user_id, f.date, f.kind, f.worked_seconds, f.ended_at IS NULL AS open,
                (SELECT n.started_at FROM workdays n
                  WHERE n.user_id = f.user_id AND n.date > f.date AND n.kind = 'work'
                  ORDER BY n.date LIMIT 1) AS next_started_at,
                (SELECT min(r.submitted_at) FROM reports r
                  WHERE r.user_id = f.user_id AND r.date = f.date) AS first_reported_at
         FROM workday_figures f
         WHERE f.user_id = ANY($1) AND f.date BETWEEN $2 AND $3",
    )
    .bind(ids)
    .bind(range.from)
    .bind(range.to)
    .fetch_all(pool)
    .await?)
}

// The rule --------------------------------------------------------------------

/// What one date was for one person.
///
/// The whole rule in one place, without a database, so every branch can be
/// held by a test. `owed` is what the calendar asks of the person on the date
/// at their share of a full day; `today` is the server's date.
pub(crate) fn cell(date: NaiveDate, today: NaiveDate, day: Option<&Recorded>, owed: i64, report: Option<Status>) -> Cell {
    let (state, filed) = match day {
        // Away first: a day of leave is excused whatever else is true of it,
        // and asks for no report - there is nothing to put a name to.
        Some(day) if !day.kind.owes_the_norm() => (CellState::Away, None),
        // An open day has no total to report yet.
        Some(day) if day.open => (CellState::Open, None),
        Some(day) => (CellState::Worked, Some(filed(day))),
        None if owed == 0 => (CellState::Free, None),
        // Come due by the norm's rule (ADR 0023): a date before today. Today
        // itself comes due when its day is closed, and a date with no day has
        // none to close.
        None if date < today => (CellState::Unrecorded, Some(Filed::Missing)),
        None => (CellState::Ahead, None),
    };

    Cell {
        date,
        state,
        kind: day.map(|day| day.kind),
        worked_seconds: day.and_then(|day| day.worked_seconds),
        norm_seconds: if state == CellState::Away { 0 } else { owed },
        filed,
        report,
    }
}

/// Whether a finished day was reported in time.
fn filed(day: &Recorded) -> Filed {
    match (day.first_reported_at, day.next_started_at) {
        (Some(reported), Some(next)) if reported > next => Filed::Late,
        (Some(_), _) => Filed::OnTime,
        (None, Some(_)) => Filed::Missing,
        (None, None) => Filed::Pending,
    }
}

/// The month's dates as the calendar has them.
fn columns(calendar: &Calendar, from: NaiveDate, to: NaiveDate, standard_hours: Decimal) -> Vec<Column> {
    from.iter_days()
        .take_while(|date| *date <= to)
        .map(|date| {
            let day = calendar.day(date);
            Column {
                date,
                hours: calendar.hours_on(date, standard_hours),
                calendar: day.map(|day| day.kind),
                note: day.and_then(|day| day.note.clone()),
                weekend: calendar::is_weekend(date),
            }
        })
        .collect()
}

/// A month for everyone `reader` may see, or - with `only` - for one person.
///
/// The visibility rule is [`team::members`]'s, applied there and nowhere
/// else: everything after it is scoped to the people it listed.
pub async fn build(pool: &PgPool, reader: &CurrentUser, month: &str, only: Option<Uuid>) -> Result<Timesheet, ApiError> {
    let (from, to) = month_bounds(month)?;
    let range = Range { from, to };

    let (members, standard_hours) = team::members(pool, reader, &range, only).await?;
    let ids: Vec<Uuid> = members.iter().map(|member| member.id).collect();

    let calendar = Calendar::load(pool, from, to).await?;
    let today = Norm::today(pool).await?;
    let day_approval = reports::approval_enabled(pool).await?;
    let dates = columns(&calendar, from, to, standard_hours);

    // Looked up by person and date: a scan per date would be people x dates x
    // days, the shape of loop that is fine for a team of five.
    let days: HashMap<(Uuid, NaiveDate), Recorded> = recorded(pool, &ids, &range)
        .await?
        .into_iter()
        .map(|day| ((day.user_id, day.date), day))
        .collect();
    let reports: HashMap<(Uuid, NaiveDate), Status> = reports::for_people(pool, &ids, &range)
        .await?
        .into_iter()
        .map(|report| ((report.user_id, report.date), report.status))
        .collect();

    let rows = members
        .into_iter()
        .map(|member| {
            let cells: Vec<Cell> = dates
                .iter()
                .map(|column| {
                    let owed = calendar.norm_seconds(column.date, standard_hours, member.work_rate);
                    let key = (member.id, column.date);
                    cell(column.date, today, days.get(&key), owed, reports.get(&key).copied())
                })
                .collect();
            let counts = Counts::of(&cells);
            Row { member, days: cells, counts }
        })
        .collect();

    Ok(Timesheet {
        month: month.to_string(),
        from,
        to,
        today,
        standard_hours,
        day_approval,
        dates,
        rows,
    })
}

// The routes ------------------------------------------------------------------

/// `GET /api/v1/team/timesheet?month=YYYY-MM`
pub async fn team_month(State(state): State<AppState>, user: CurrentUser, Query(query): Query<MonthQuery>) -> Result<impl IntoResponse, ApiError> {
    require_manager_or_admin(&user)?;
    Ok(Json(build(&state.pool, &user, &query.month, None).await?))
}

/// `GET /api/v1/me/timesheet?month=YYYY-MM`
///
/// The same month with one row - the reader's. What `kasl timesheet` files
/// from the person's own machine, as the server has it.
pub async fn my_month(State(state): State<AppState>, user: CurrentUser, Query(query): Query<MonthQuery>) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(build(&state.pool, &user, &query.month, Some(user.user_id)).await?))
}

/// `GET /api/v1/team/timesheet.xlsx?month=YYYY-MM`
pub async fn team_workbook(State(state): State<AppState>, user: CurrentUser, Query(query): Query<MonthQuery>) -> Result<Response, ApiError> {
    require_manager_or_admin(&user)?;
    let timesheet = build(&state.pool, &user, &query.month, None).await?;
    let file = workbook::write(&timesheet)?;

    // Other people's hours leaving the server with the reader: the read the
    // audit log keeps (ADR 0023).
    audit::Entry::new(audit::action::HOURS_EXPORTED)
        .by(user.user_id)
        .by_email(&user.email)
        .with(serde_json::json!({
            "file": "timesheet",
            "from": timesheet.from,
            "to": timesheet.to,
            "people": timesheet.rows.len(),
        }))
        .record(&state.pool)
        .await;

    Ok(export::download(
        file,
        workbook::CONTENT_TYPE,
        &format!("kasl-team-timesheet-{}.xlsx", timesheet.month),
    ))
}

/// `GET /api/v1/me/timesheet.xlsx?month=YYYY-MM`
pub async fn my_workbook(State(state): State<AppState>, user: CurrentUser, Query(query): Query<MonthQuery>) -> Result<Response, ApiError> {
    let timesheet = build(&state.pool, &user, &query.month, Some(user.user_id)).await?;
    let file = workbook::write(&timesheet)?;
    Ok(export::download(
        file,
        workbook::CONTENT_TYPE,
        &format!("kasl-me-timesheet-{}.xlsx", timesheet.month),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> NaiveDate {
        text.parse().expect("a test date")
    }

    fn at(text: &str) -> DateTime<Utc> {
        text.parse().expect("a test instant")
    }

    fn bounds(month: &str) -> (NaiveDate, NaiveDate) {
        month_bounds(month).expect("a valid month")
    }

    const EIGHT_HOURS: i64 = 8 * 3600;

    /// A finished day of work on the 14th, its next working day beginning on
    /// the 15th at nine.
    fn worked() -> Recorded {
        Recorded {
            user_id: Uuid::nil(),
            date: date("2026-09-14"),
            kind: WorkdayKind::Work,
            worked_seconds: Some(27_000),
            open: false,
            next_started_at: Some(at("2026-09-15T09:00:00Z")),
            first_reported_at: None,
        }
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

    #[test]
    fn a_report_before_the_next_day_began_is_on_time_and_one_after_is_late() {
        let today = date("2026-09-20");
        let mut day = worked();

        day.first_reported_at = Some(at("2026-09-14T18:05:00Z"));
        assert_eq!(cell(day.date, today, Some(&day), EIGHT_HOURS, None).filed, Some(Filed::OnTime));

        // A minute into the next day is late: the deadline is a moment, and a
        // grace period would be a setting nobody asked for.
        day.first_reported_at = Some(at("2026-09-15T09:01:00Z"));
        assert_eq!(cell(day.date, today, Some(&day), EIGHT_HOURS, None).filed, Some(Filed::Late));
    }

    #[test]
    fn an_unreported_day_is_missing_once_the_next_has_begun_and_pending_until() {
        let today = date("2026-09-20");
        let mut day = worked();
        assert_eq!(cell(day.date, today, Some(&day), EIGHT_HOURS, None).filed, Some(Filed::Missing));

        // Friday's day, with the weekend in between and Monday not begun:
        // there is still time, whatever the calendar says about Saturday.
        day.next_started_at = None;
        assert_eq!(cell(day.date, today, Some(&day), EIGHT_HOURS, None).filed, Some(Filed::Pending));

        // And reported with nothing begun since, it is on time.
        day.first_reported_at = Some(at("2026-09-19T10:00:00Z"));
        assert_eq!(cell(day.date, today, Some(&day), EIGHT_HOURS, None).filed, Some(Filed::OnTime));
    }

    #[test]
    fn a_day_away_owes_nothing_and_asks_for_no_report() {
        let mut day = worked();
        day.kind = WorkdayKind::Vacation;
        day.worked_seconds = Some(0);
        let away = cell(day.date, date("2026-09-20"), Some(&day), EIGHT_HOURS, None);

        assert_eq!(away.state, CellState::Away);
        assert_eq!(away.kind, Some(WorkdayKind::Vacation));
        assert_eq!(away.norm_seconds, 0, "leave is excused (ADR 0017)");
        assert_eq!(away.filed, None, "nothing was worked, so nothing is reported");
    }

    #[test]
    fn an_open_day_has_no_total_and_no_report_yet() {
        let mut day = worked();
        day.open = true;
        day.worked_seconds = None;
        let open = cell(day.date, date("2026-09-14"), Some(&day), EIGHT_HOURS, None);

        assert_eq!(open.state, CellState::Open);
        assert_eq!(open.worked_seconds, None);
        assert_eq!(open.filed, None);
        assert_eq!(open.norm_seconds, EIGHT_HOURS, "still owed - it is a working day");
    }

    #[test]
    fn a_date_with_nothing_is_unrecorded_once_due_ahead_before_and_free_when_owed_nothing() {
        let today = date("2026-09-16");

        let gone = cell(date("2026-09-15"), today, None, EIGHT_HOURS, None);
        assert_eq!((gone.state, gone.filed), (CellState::Unrecorded, Some(Filed::Missing)));
        assert_eq!(gone.worked_seconds, None, "absent, never a zero (ADR 0015)");

        // Today with nothing recorded has not come due: the morning is not a
        // day missed (ADR 0023).
        let now = cell(today, today, None, EIGHT_HOURS, None);
        assert_eq!((now.state, now.filed), (CellState::Ahead, None));

        let later = cell(date("2026-09-17"), today, None, EIGHT_HOURS, None);
        assert_eq!(later.state, CellState::Ahead);

        // A Saturday, a holiday, a rate of zero: nothing was owed, so nothing
        // is missing - in the past as much as ahead.
        let rest = cell(date("2026-09-12"), today, None, 0, None);
        assert_eq!((rest.state, rest.filed, rest.norm_seconds), (CellState::Free, None, 0));
    }

    #[test]
    fn a_day_worked_on_a_date_owed_nothing_is_still_worked_and_still_reported() {
        // Saturday work: it owes nothing by the calendar, and it is a day the
        // person will be asked to put their name to like any other.
        let mut day = worked();
        day.date = date("2026-09-12");
        let saturday = cell(day.date, date("2026-09-20"), Some(&day), 0, Some(Status::Submitted));

        assert_eq!(saturday.state, CellState::Worked);
        assert_eq!(saturday.norm_seconds, 0);
        assert_eq!(saturday.filed, Some(Filed::Missing), "no report arrived before Monday began");
        assert_eq!(saturday.report, Some(Status::Submitted), "the newest report's status is carried as is");
    }

    #[test]
    fn counts_tell_leave_by_kind_and_reports_by_timeliness() {
        let today = date("2026-09-20");
        let mut sick = worked();
        sick.kind = WorkdayKind::Sick;
        let mut late = worked();
        late.first_reported_at = Some(at("2026-09-16T08:00:00Z"));

        let cells = vec![
            cell(date("2026-09-14"), today, Some(&sick), EIGHT_HOURS, None),
            cell(date("2026-09-14"), today, Some(&late), EIGHT_HOURS, Some(Status::Approved)),
            cell(date("2026-09-15"), today, None, EIGHT_HOURS, None),
            cell(date("2026-09-19"), today, None, 0, None),
        ];
        let counts = Counts::of(&cells);

        assert_eq!(
            counts,
            Counts {
                sick: 1,
                unrecorded: 1,
                late: 1,
                missing: 1,
                approved: 1,
                ..Counts::default()
            }
        );
    }

    #[test]
    fn the_columns_carry_the_calendar() {
        let calendar = Calendar::from_days(vec![
            calendar::CalendarDay {
                date: date("2026-11-04"),
                kind: CalendarDayKind::Holiday,
                note: Some("Unity Day".into()),
            },
            calendar::CalendarDay {
                date: date("2026-11-07"),
                kind: CalendarDayKind::WorkingWeekend,
                note: None,
            },
        ]);
        let (from, to) = bounds("2026-11");
        let columns = columns(&calendar, from, to, Decimal::from(8));

        assert_eq!(columns.len(), 30, "every date of November, and no more");
        let on = |text: &str| columns.iter().find(|column| column.date == date(text)).expect("the date is a column");

        let holiday = on("2026-11-04");
        assert_eq!(
            (holiday.hours, holiday.calendar, holiday.note.as_deref()),
            (Decimal::ZERO, Some(CalendarDayKind::Holiday), Some("Unity Day"))
        );
        let saturday = on("2026-11-07");
        assert!(saturday.weekend, "a working weekend is still a Saturday");
        assert_eq!(saturday.hours, Decimal::from(8), "and a full day");
        assert_eq!(on("2026-11-08").hours, Decimal::ZERO);
    }

    #[test]
    fn the_wire_names_are_snake_case() {
        // Part of the contract with the web UI and with `kasl timesheet`.
        assert_eq!(serde_json::to_string(&CellState::Unrecorded).unwrap(), "\"unrecorded\"");
        assert_eq!(serde_json::to_string(&Filed::OnTime).unwrap(), "\"on_time\"");
    }
}
