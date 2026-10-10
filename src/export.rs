//! Hours leaving the server as files: a range as a spreadsheet (ADR 0023).
//!
//! Two tables, each in two formats:
//!
//! * **Summary** - a row per person: the figures the team table shows for the
//!   same range, read through the same function ([`team::members`]), so a
//!   spreadsheet and the screen it was downloaded from cannot disagree.
//! * **Days** - a row per person and date: every date that was recorded, and
//!   every date that was due without being recorded. Both kinds, because then
//!   the days add up to the summary - the worked hours and the norm alike -
//!   and a reader reconciling the two finds nothing missing.
//!
//! **CSV is for programs, the workbook is for people.** The CSV headers are
//! `snake_case` names a script can index, and a file holds one table; the
//! workbook holds both tables as sheets, with dates as dates and hours as
//! numbers a spreadsheet can sum. Both are built from one list of columns,
//! so the two formats cannot grow apart.
//!
//! Exporting other people's hours is recorded in the audit log: it is the one
//! read that leaves the server with the data in hand. Exporting your own is
//! not - nobody else's record is involved.

use std::collections::HashMap;

use axum::{
    extract::{Query, State},
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use rust_decimal::{Decimal, RoundingStrategy, prelude::ToPrimitive};
use rust_xlsxwriter::{DocProperties, Format, Workbook, Worksheet, XlsxError};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    admin::require_manager_or_admin,
    app::AppState,
    audit,
    calendar::{Calendar, WorkdayKind},
    error::ApiError,
    login::CurrentUser,
    me::{self, Range},
    reports::{self, Status},
    team::{self, Member},
};

/// Seconds in an hour: every duration leaves as hours.
const SECONDS_PER_HOUR: i64 = 3600;

/// How hours are written in a CSV: two decimal places, which is what payroll
/// and every tracker's export use. The workbook keeps the exact figure and only
/// shows two places, so its sums are exact.
const CSV_HOUR_PLACES: u32 = 2;

// The tables ------------------------------------------------------------------

/// One column, named for each audience.
struct Column {
    /// The CSV header: a name a script indexes by.
    csv: &'static str,
    /// The workbook header: words for a person.
    sheet: &'static str,
}

/// A value, typed by what it means rather than how it is printed - each
/// format decides how a date or an hour figure looks in it.
#[derive(Debug, Clone, PartialEq)]
enum Cell {
    Text(String),
    Count(i64),
    /// A duration, in seconds. Leaves as hours.
    Hours(i64),
    /// A share of a full day.
    Share(Decimal),
    Date(NaiveDate),
    Instant(DateTime<Utc>),
    Flag(bool),
    /// Nothing to say: an open day's total, a date with no record.
    Empty,
}

/// A table: its columns and its rows, cell for column.
struct Table {
    /// The sheet's name in the workbook.
    name: &'static str,
    columns: &'static [Column],
    rows: Vec<Vec<Cell>>,
}

const SUMMARY: &[Column] = &[
    Column {
        csv: "person",
        sheet: "Person",
    },
    Column { csv: "email", sheet: "Email" },
    Column {
        csv: "department",
        sheet: "Department",
    },
    Column {
        csv: "active",
        sheet: "Active",
    },
    Column {
        csv: "work_rate",
        sheet: "Share of a full day",
    },
    Column {
        csv: "days_worked",
        sheet: "Days worked",
    },
    Column {
        csv: "days_away",
        sheet: "Days away",
    },
    Column {
        csv: "worked_hours",
        sheet: "Worked (h)",
    },
    Column {
        csv: "paused_hours",
        sheet: "Paused (h)",
    },
    Column {
        csv: "norm_hours",
        sheet: "Norm (h)",
    },
];

const DAYS: &[Column] = &[
    Column {
        csv: "person",
        sheet: "Person",
    },
    Column { csv: "email", sheet: "Email" },
    Column {
        csv: "department",
        sheet: "Department",
    },
    Column { csv: "date", sheet: "Date" },
    Column { csv: "kind", sheet: "Kind" },
    Column {
        csv: "started_at",
        sheet: "Started (UTC)",
    },
    Column {
        csv: "ended_at",
        sheet: "Ended (UTC)",
    },
    Column {
        csv: "worked_hours",
        sheet: "Worked (h)",
    },
    Column {
        csv: "paused_hours",
        sheet: "Paused (h)",
    },
    Column {
        csv: "norm_hours",
        sheet: "Norm (h)",
    },
    Column {
        csv: "report",
        sheet: "Report",
    },
];

/// The summary: one row per person, in the team table's order.
fn summary_table(members: &[Member]) -> Table {
    let rows = members
        .iter()
        .map(|member| {
            vec![
                Cell::Text(member.display_name.clone()),
                Cell::Text(member.email.clone()),
                member.department.clone().map_or(Cell::Empty, Cell::Text),
                Cell::Flag(member.active),
                Cell::Share(member.work_rate),
                Cell::Count(member.days_recorded),
                Cell::Count(member.days_away),
                Cell::Hours(member.worked_seconds),
                Cell::Hours(member.paused_seconds),
                Cell::Hours(member.norm_seconds),
            ]
        })
        .collect();
    Table {
        name: "Summary",
        columns: SUMMARY,
        rows,
    }
}

/// A stored day, as the days table reads it from `workday_figures`.
#[derive(Debug, sqlx::FromRow)]
struct DayRow {
    user_id: Uuid,
    date: NaiveDate,
    kind: WorkdayKind,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
    worked_seconds: Option<i64>,
    paused_seconds: i64,
}

/// The days: a row per person and date that was either recorded or due.
///
/// A date that was neither - a weekend nobody worked, a holiday - has nothing
/// to say and is left out. A date that was due and not recorded is a row with
/// its norm and nothing else: absent data stays absent (ADR 0015), and the norm
/// beside it is what makes the days add up to the summary.
async fn days_table(pool: &PgPool, members: &[Member], range: &Range) -> Result<Table, ApiError> {
    let ids: Vec<Uuid> = members.iter().map(|member| member.id).collect();

    let days: Vec<DayRow> = sqlx::query_as(
        "SELECT user_id, date, kind, started_at, ended_at, worked_seconds, paused_seconds
         FROM workday_figures
         WHERE user_id = ANY($1) AND date BETWEEN $2 AND $3
         ORDER BY user_id, date",
    )
    .bind(&ids)
    .bind(range.from)
    .bind(range.to)
    .fetch_all(pool)
    .await?;
    let reports = reports::for_people(pool, &ids, range).await?;
    let calendar = Calendar::load(pool, range.from, range.to).await?;
    let standard_hours = crate::calendar::Norm::standard_hours(pool).await?;

    // Looked up by person and date. A scan per date would be people x dates x
    // days, and a team's year is a hundred million comparisons.
    let days: HashMap<(Uuid, NaiveDate), &DayRow> = days.iter().map(|day| ((day.user_id, day.date), day)).collect();
    let reports: HashMap<(Uuid, NaiveDate), Status> = reports.iter().map(|report| ((report.user_id, report.date), report.status)).collect();

    let mut rows = Vec::new();
    for member in members {
        let mut date = range.from;
        while date <= range.to {
            let day = days.get(&(member.id, date)).copied();
            let report = reports.get(&(member.id, date)).copied();
            // The same rule the personal page and the summary follow: leave owes
            // nothing, and a date with no record owes what the calendar says.
            let owes = day.is_none_or(|day| day.kind.owes_the_norm());
            let norm = if owes {
                calendar.norm_seconds(date, standard_hours, member.work_rate)
            } else {
                0
            };

            if day.is_some() || norm > 0 {
                rows.push(day_row(member, date, day, norm, report));
            }

            let Some(next) = date.succ_opt() else { break };
            date = next;
        }
    }

    Ok(Table {
        name: "Days",
        columns: DAYS,
        rows,
    })
}

fn day_row(member: &Member, date: NaiveDate, day: Option<&DayRow>, norm: i64, report: Option<Status>) -> Vec<Cell> {
    let mut row = vec![
        Cell::Text(member.display_name.clone()),
        Cell::Text(member.email.clone()),
        member.department.clone().map_or(Cell::Empty, Cell::Text),
        Cell::Date(date),
    ];
    match day {
        Some(day) => row.extend([
            Cell::Text(kind_name(day.kind).to_string()),
            Cell::Instant(day.started_at),
            day.ended_at.map_or(Cell::Empty, Cell::Instant),
            // An open day has no total yet, and a partial one would read as a
            // short day.
            day.worked_seconds.map_or(Cell::Empty, Cell::Hours),
            Cell::Hours(day.paused_seconds),
        ]),
        None => row.extend([Cell::Empty, Cell::Empty, Cell::Empty, Cell::Empty, Cell::Empty]),
    }
    row.push(Cell::Hours(norm));
    row.push(report.map_or(Cell::Empty, |status| Cell::Text(status_name(status).to_string())));
    row
}

/// A day's kind in the words the API uses for it.
fn kind_name(kind: WorkdayKind) -> &'static str {
    match kind {
        WorkdayKind::Work => "work",
        WorkdayKind::Vacation => "vacation",
        WorkdayKind::Sick => "sick",
        WorkdayKind::DayOff => "day_off",
    }
}

/// A report's status in the words the API uses for it.
fn status_name(status: Status) -> &'static str {
    match status {
        Status::Submitted => "submitted",
        Status::Approved => "approved",
        Status::Returned => "returned",
        Status::Changed => "changed",
    }
}

// CSV -------------------------------------------------------------------------

/// One table as RFC 4180 CSV: UTF-8, comma-separated, CRLF line ends, a
/// header row.
///
/// No byte-order mark. The file is for programs, which a BOM trips; a person
/// who wants Excel gets the workbook, which opens in any language with no
/// guessing about encodings.
fn to_csv(table: &Table) -> String {
    let mut out = String::new();
    let header: Vec<String> = table.columns.iter().map(|column| column.csv.to_string()).collect();
    push_csv_line(&mut out, &header);
    for row in &table.rows {
        let fields: Vec<String> = row.iter().map(csv_field).collect();
        push_csv_line(&mut out, &fields);
    }
    out
}

fn push_csv_line(out: &mut String, fields: &[String]) {
    for (at, field) in fields.iter().enumerate() {
        if at > 0 {
            out.push(',');
        }
        if field.contains([',', '"', '\r', '\n']) {
            out.push('"');
            out.push_str(&field.replace('"', "\"\""));
            out.push('"');
        } else {
            out.push_str(field);
        }
    }
    out.push_str("\r\n");
}

fn csv_field(cell: &Cell) -> String {
    match cell {
        Cell::Text(text) => defuse(text),
        Cell::Count(count) => count.to_string(),
        Cell::Hours(seconds) => hours_text(*seconds),
        Cell::Share(share) => share.normalize().to_string(),
        Cell::Date(date) => date.to_string(),
        Cell::Instant(instant) => instant.to_rfc3339_opts(SecondsFormat::Secs, true),
        Cell::Flag(flag) => flag.to_string(),
        Cell::Empty => String::new(),
    }
}

/// Seconds as hours to two places, rounded half away from zero.
///
/// Through a decimal rather than a float: 27 000 seconds is 7.50 hours, and a
/// float on its way there can arrive at 7.4999999 and print 7.49.
fn hours_text(seconds: i64) -> String {
    let hours = Decimal::from(seconds) / Decimal::from(SECONDS_PER_HOUR);
    format!(
        "{:.places$}",
        hours.round_dp_with_strategy(CSV_HOUR_PLACES, RoundingStrategy::MidpointAwayFromZero),
        places = CSV_HOUR_PLACES as usize
    )
}

/// Keeps a spreadsheet from running text as a formula.
///
/// A CSV is opened in a spreadsheet as often as it is parsed, and a name that
/// begins with `=`, `+`, `-` or `@` becomes a formula there - the injection
/// OWASP describes. Names and departments are typed by people, so the text
/// fields are prefixed with an apostrophe when they start that way; numbers
/// are never touched, and a negative one stays a number. The workbook needs
/// none of this: its strings are written as strings.
fn defuse(text: &str) -> String {
    if text.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{text}")
    } else {
        text.to_string()
    }
}

// The workbook ----------------------------------------------------------------

/// Tables as sheets of one workbook.
fn to_xlsx(tables: &[Table], title: &str) -> Result<Vec<u8>, XlsxError> {
    let mut workbook = Workbook::new();
    workbook.set_properties(&DocProperties::new().set_title(title));

    let header = Format::new().set_bold();
    let hours = Format::new().set_num_format("0.00");
    let share = Format::new().set_num_format("0.##");
    let date = Format::new().set_num_format("yyyy-mm-dd");
    let instant = Format::new().set_num_format("yyyy-mm-dd hh:mm");

    for table in tables {
        let mut sheet = Worksheet::new();
        sheet.set_name(table.name)?;

        for (col, column) in (0u16..).zip(table.columns) {
            sheet.write_string_with_format(0, col, column.sheet, &header)?;
        }
        for (row, cells) in (1u32..).zip(&table.rows) {
            for (col, cell) in (0u16..).zip(cells) {
                match cell {
                    Cell::Text(text) => {
                        sheet.write_string(row, col, text)?;
                    }
                    Cell::Count(count) => {
                        sheet.write_number(row, col, *count as f64)?;
                    }
                    // The exact figure, shown to two places: a column of these
                    // sums to what the summary says, to the second.
                    Cell::Hours(seconds) => {
                        sheet.write_number_with_format(row, col, *seconds as f64 / SECONDS_PER_HOUR as f64, &hours)?;
                    }
                    Cell::Share(value) => {
                        sheet.write_number_with_format(row, col, value.to_f64().unwrap_or(0.0), &share)?;
                    }
                    Cell::Date(value) => {
                        sheet.write_date_with_format(row, col, value, &date)?;
                    }
                    // A cell holds no zone. The header says UTC, and the value
                    // is the UTC wall clock - the date column beside it is the
                    // employee's own (ADR 0003).
                    Cell::Instant(value) => {
                        sheet.write_datetime_with_format(row, col, value.naive_utc(), &instant)?;
                    }
                    Cell::Flag(flag) => {
                        sheet.write_boolean(row, col, *flag)?;
                    }
                    Cell::Empty => {}
                }
            }
        }

        // The header stays in view and every column can be filtered: the two
        // things anyone does first with a sheet of a hundred rows.
        let last_col = table.columns.len().saturating_sub(1) as u16;
        sheet.set_freeze_panes(1, 0)?;
        sheet.autofilter(0, 0, table.rows.len() as u32, last_col)?;
        sheet.autofit();

        workbook.push_worksheet(sheet);
    }

    workbook.save_to_buffer()
}

// The routes ------------------------------------------------------------------

/// Whose hours a file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Subject {
    /// Everyone the reader may see.
    Team,
    /// The reader alone.
    Me,
}

impl Subject {
    /// The word a file name starts with.
    fn slug(self) -> &'static str {
        match self {
            Self::Team => "team",
            Self::Me => "me",
        }
    }
}

/// Which file is being asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum File {
    /// Both tables, as a workbook.
    Workbook,
    SummaryCsv,
    DaysCsv,
}

impl File {
    /// How the audit log names it.
    fn name(self) -> &'static str {
        match self {
            Self::Workbook => "workbook",
            Self::SummaryCsv => "summary.csv",
            Self::DaysCsv => "days.csv",
        }
    }
}

/// `GET /api/v1/team/export.xlsx`
pub async fn team_workbook(state: State<AppState>, user: CurrentUser, range: Query<Range>) -> Result<Response, ApiError> {
    export(state, user, range, Subject::Team, File::Workbook).await
}

/// `GET /api/v1/team/export/summary.csv`
pub async fn team_summary(state: State<AppState>, user: CurrentUser, range: Query<Range>) -> Result<Response, ApiError> {
    export(state, user, range, Subject::Team, File::SummaryCsv).await
}

/// `GET /api/v1/team/export/days.csv`
pub async fn team_days(state: State<AppState>, user: CurrentUser, range: Query<Range>) -> Result<Response, ApiError> {
    export(state, user, range, Subject::Team, File::DaysCsv).await
}

/// `GET /api/v1/me/export.xlsx`
pub async fn my_workbook(state: State<AppState>, user: CurrentUser, range: Query<Range>) -> Result<Response, ApiError> {
    export(state, user, range, Subject::Me, File::Workbook).await
}

/// `GET /api/v1/me/export/summary.csv`
pub async fn my_summary(state: State<AppState>, user: CurrentUser, range: Query<Range>) -> Result<Response, ApiError> {
    export(state, user, range, Subject::Me, File::SummaryCsv).await
}

/// `GET /api/v1/me/export/days.csv`
pub async fn my_days(state: State<AppState>, user: CurrentUser, range: Query<Range>) -> Result<Response, ApiError> {
    export(state, user, range, Subject::Me, File::DaysCsv).await
}

/// Builds the file and answers it as a download.
async fn export(State(state): State<AppState>, user: CurrentUser, Query(range): Query<Range>, subject: Subject, file: File) -> Result<Response, ApiError> {
    if subject == Subject::Team {
        require_manager_or_admin(&user)?;
    }
    me::validate_range(&range)?;

    let only = (subject == Subject::Me).then_some(user.user_id);
    let (members, _) = team::members(&state.pool, &user, &range, only).await?;

    let stem = format!("kasl-{}-{}-to-{}", subject.slug(), range.from, range.to);
    let (body, content_type, file_name) = match file {
        File::Workbook => {
            let tables = [summary_table(&members), days_table(&state.pool, &members, &range).await?];
            let title = format!("Hours, {} to {}", range.from, range.to);
            let bytes = to_xlsx(&tables, &title).map_err(|error| anyhow::anyhow!("could not write the workbook: {error}"))?;
            (
                bytes,
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                format!("{stem}.xlsx"),
            )
        }
        File::SummaryCsv => (
            to_csv(&summary_table(&members)).into_bytes(),
            "text/csv; charset=utf-8",
            format!("{stem}-summary.csv"),
        ),
        File::DaysCsv => (
            to_csv(&days_table(&state.pool, &members, &range).await?).into_bytes(),
            "text/csv; charset=utf-8",
            format!("{stem}-days.csv"),
        ),
    };

    if subject == Subject::Team {
        audit::Entry::new(audit::action::HOURS_EXPORTED)
            .by(user.user_id)
            .by_email(&user.email)
            .with(serde_json::json!({
                "file": file.name(),
                "from": range.from,
                "to": range.to,
                "people": members.len(),
            }))
            .record(&state.pool)
            .await;
    }

    Ok(download(body, content_type, &file_name))
}

/// A file the browser saves rather than shows.
///
/// `no-store`: the body is people's working hours, and a shared machine's
/// cache is not where a copy of them should outlive the download.
fn download(body: Vec<u8>, content_type: &'static str, file_name: &str) -> Response {
    let disposition = format!("attachment; filename=\"{file_name}\"");
    let mut response = body.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    // The name is built from dates and fixed words, never from anything typed.
    if let Ok(value) = HeaderValue::from_str(&disposition) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hours_round_half_away_from_zero_through_a_decimal() {
        assert_eq!(hours_text(27_000), "7.50");
        assert_eq!(hours_text(0), "0.00");
        // 18 seconds is 0.005 h exactly: the midpoint goes up, not to the even
        // neighbour - the rounding a payroll clerk does by hand.
        assert_eq!(hours_text(18), "0.01");
        assert_eq!(hours_text(17), "0.00");
        assert_eq!(hours_text(-18), "-0.01");
    }

    #[test]
    fn a_field_is_quoted_only_when_it_must_be() {
        let mut out = String::new();
        push_csv_line(
            &mut out,
            &["plain".into(), "a, b".into(), "say \"hi\"".into(), "two\nlines".into(), String::new()],
        );
        assert_eq!(out, "plain,\"a, b\",\"say \"\"hi\"\"\",\"two\nlines\",\r\n");
    }

    #[test]
    fn text_that_a_spreadsheet_would_run_is_defused_and_numbers_are_not() {
        for dangerous in ["=HYPERLINK(\"x\")", "+1", "-2", "@SUM(A1)", "\tlead"] {
            assert!(csv_field(&Cell::Text(dangerous.into())).starts_with('\''), "{dangerous:?} should be defused");
        }
        assert_eq!(csv_field(&Cell::Text("Ada Lovelace".into())), "Ada Lovelace");
        // A figure that is negative is a number, not an injection.
        assert_eq!(csv_field(&Cell::Hours(-3600)), "-1.00");
    }

    #[test]
    fn every_value_has_a_csv_form() {
        let instant: DateTime<Utc> = "2026-10-07T12:03:09Z".parse().unwrap();
        assert_eq!(csv_field(&Cell::Instant(instant)), "2026-10-07T12:03:09Z");
        assert_eq!(csv_field(&Cell::Date("2026-10-07".parse().unwrap())), "2026-10-07");
        assert_eq!(csv_field(&Cell::Share(Decimal::new(50, 2))), "0.5");
        assert_eq!(csv_field(&Cell::Share(Decimal::ONE)), "1");
        assert_eq!(csv_field(&Cell::Flag(false)), "false");
        assert_eq!(csv_field(&Cell::Count(3)), "3");
        assert_eq!(csv_field(&Cell::Empty), "");
    }

    #[test]
    fn a_csv_has_its_header_and_one_line_per_row() {
        let table = Table {
            name: "Summary",
            columns: &[
                Column {
                    csv: "person",
                    sheet: "Person",
                },
                Column {
                    csv: "worked_hours",
                    sheet: "Worked (h)",
                },
            ],
            rows: vec![vec![Cell::Text("Ada".into()), Cell::Hours(5400)]],
        };
        assert_eq!(to_csv(&table), "person,worked_hours\r\nAda,1.50\r\n");
    }

    #[test]
    fn a_workbook_is_a_zip_with_a_sheet_per_table() {
        let tables = [
            Table {
                name: "Summary",
                columns: SUMMARY,
                rows: vec![],
            },
            Table {
                name: "Days",
                columns: DAYS,
                rows: vec![vec![
                    Cell::Text("Ada".into()),
                    Cell::Text("ada@example.test".into()),
                    Cell::Empty,
                    Cell::Date("2026-10-07".parse().unwrap()),
                    Cell::Text("work".into()),
                    Cell::Instant("2026-10-07T12:00:00Z".parse().unwrap()),
                    Cell::Empty,
                    Cell::Empty,
                    Cell::Hours(0),
                    Cell::Hours(28_800),
                    Cell::Empty,
                ]],
            },
        ];
        let bytes = to_xlsx(&tables, "Hours").expect("the workbook should write");
        // A zip's local file header. What the parts inside say is the
        // integration tests' business; here it is enough that one was made.
        assert_eq!(&bytes[..4], b"PK\x03\x04");
    }

    fn member() -> Member {
        Member {
            id: Uuid::nil(),
            display_name: "Ada".into(),
            email: "ada@example.test".into(),
            department: None,
            active: true,
            days_recorded: 0,
            worked_seconds: 0,
            paused_seconds: 0,
            last_day: None,
            day_open: false,
            last_seen_at: None,
            agents: 0,
            work_rate: Decimal::ONE,
            norm_seconds: 0,
            days_away: 0,
        }
    }

    #[test]
    fn every_row_has_a_cell_per_column() {
        // A row with a cell too many or too few would shift every column after
        // it under the wrong header, in both formats at once.
        let summary = summary_table(&[member()]);
        assert_eq!(summary.rows[0].len(), summary.columns.len());

        let date = "2026-10-07".parse().unwrap();
        assert_eq!(day_row(&member(), date, None, 28_800, None).len(), DAYS.len(), "a date with no record");
        let day = DayRow {
            user_id: Uuid::nil(),
            date,
            kind: WorkdayKind::Work,
            started_at: "2026-10-07T12:00:00Z".parse().unwrap(),
            ended_at: None,
            worked_seconds: None,
            paused_seconds: 0,
        };
        assert_eq!(
            day_row(&member(), date, Some(&day), 28_800, Some(Status::Submitted)).len(),
            DAYS.len(),
            "a recorded day"
        );
    }
}
