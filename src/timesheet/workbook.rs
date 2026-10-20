//! The timesheet as a workbook: the grid a company files, for people to read,
//! print and sign (ADR 0024).
//!
//! Two sheets over one frame. **Timesheet** is the month in hours: a row per
//! person, a column per date, the hours of each day worked, a letter for each
//! day away, and the month's totals at the end of the row. **Reports** is the
//! same grid answering a second question - was each day reported, and in
//! time. The frame - the dates, their weekdays, what each one is worth at a
//! full day, grey columns where nothing is owed - is written once for both, so
//! the two sheets cannot put a date in different places.
//!
//! A grid rather than the days export's long table: a long table is what a
//! program reads, and the days CSV already is one. A grid is what a person
//! checks a month against, prints, and signs.

use chrono::Datelike;
use rust_decimal::{Decimal, prelude::ToPrimitive};
use rust_xlsxwriter::{Color, DocProperties, Format, FormatAlign, FormatPattern, Workbook, Worksheet, XlsxError};

use super::{CellState, Column, Filed, Row, Timesheet};
use crate::{
    calendar::{CalendarDayKind, WorkdayKind},
    error::ApiError,
    reports::Status,
};

/// The media type of a workbook.
pub const CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

const SECONDS_PER_HOUR: f64 = 3600.0;

/// The row of day numbers. Above it, the title and a line under it.
const HEAD: u32 = 3;
/// The row of weekdays.
const WEEKDAYS: u32 = 4;
/// The row of what each date is worth at a full day: the plan.
const PLAN: u32 = 5;
/// The first person.
const FIRST_PERSON: u32 = 6;

/// The person and their department come before the dates.
const FIRST_DATE: u16 = 2;

/// A rest day's column: a weekend nobody moved, or a holiday.
const REST: u32 = 0xEDEDED;
/// Something to look at: a date owed and unrecorded, a report returned.
const ATTENTION: u32 = 0xFCE4D6;
/// A day approved, still standing for the figures it was approved at.
const APPROVED: u32 = 0xE2EFDA;
const INK_OK: u32 = 0x2E7D32;
const INK_WARN: u32 = 0x9C5700;
const INK_BAD: u32 = 0xC00000;
const INK_FAINT: u32 = 0x808080;

/// Writes the month as a workbook.
pub fn write(timesheet: &Timesheet) -> Result<Vec<u8>, ApiError> {
    Ok(build(timesheet).map_err(|error| anyhow::anyhow!("could not write the timesheet: {error}"))?)
}

fn build(timesheet: &Timesheet) -> Result<Vec<u8>, XlsxError> {
    let month = timesheet.from.format("%B %Y").to_string();
    let mut workbook = Workbook::new();
    workbook.set_properties(&DocProperties::new().set_title(format!("Timesheet, {month}")));

    workbook.push_worksheet(hours_sheet(timesheet, &month)?);
    workbook.push_worksheet(reports_sheet(timesheet, &month)?);
    workbook.save_to_buffer()
}

// The frame -------------------------------------------------------------------

/// The totals at the end of each row, by their headers.
struct Totals {
    headers: &'static [&'static str],
}

/// Writes what both sheets share: the title, the dates and their weekdays, the
/// plan row, the column widths, and how the sheet prints.
fn frame(sheet: &mut Worksheet, timesheet: &Timesheet, title: &str, totals: &Totals) -> Result<(), XlsxError> {
    let bold = Format::new().set_bold();
    let heading = Format::new().set_bold().set_font_size(14);
    let faint = Format::new().set_italic().set_font_color(Color::RGB(INK_FAINT));
    let centred = Format::new().set_align(FormatAlign::Center);

    sheet.write_string_with_format(0, 0, title, &heading)?;
    sheet.write_string_with_format(
        1,
        0,
        format!("A full day here is {} h. Read on {}.", hours_label(timesheet.standard_hours), timesheet.today),
        &faint,
    )?;

    sheet.write_string_with_format(HEAD, 0, "Person", &bold)?;
    sheet.write_string_with_format(HEAD, 1, "Department", &bold)?;
    sheet.write_string_with_format(PLAN, 0, "Full day (h)", &faint)?;

    let day_number = bold.clone().set_align(FormatAlign::Center);
    for (col, column) in date_columns(&timesheet.dates) {
        let rest = shaded(&centred, column);
        sheet.write_number_with_format(HEAD, col, f64::from(column.date.day()), &shaded(&day_number, column))?;
        sheet.write_string_with_format(WEEKDAYS, col, column.date.format("%a").to_string(), &rest)?;
        if column.hours.is_zero() {
            sheet.write_blank(PLAN, col, &rest)?;
        } else {
            sheet.write_number_with_format(PLAN, col, column.hours.to_f64().unwrap_or(0.0), &rest)?;
        }
        sheet.set_column_width(col, 5)?;
    }

    let first_total = totals_start(timesheet);
    for (col, header) in (first_total..).zip(totals.headers) {
        sheet.write_string_with_format(HEAD, col, *header, &bold.clone().set_text_wrap())?;
        sheet.set_column_width(col, 11)?;
    }

    sheet.set_column_width(0, 26)?;
    sheet.set_column_width(1, 18)?;

    // The names and the dates stay in view while the grid scrolls, and on
    // paper the dates repeat on every page: a month of a big team does not fit
    // on one sheet, and page three without its header is a page of numbers.
    sheet.set_freeze_panes(FIRST_PERSON, FIRST_DATE)?;
    sheet.set_repeat_rows(HEAD, PLAN)?;
    sheet.set_landscape();
    sheet.set_print_fit_to_pages(1, 0);
    Ok(())
}

/// The dates with the sheet column each one is written in.
fn date_columns(dates: &[Column]) -> impl Iterator<Item = (u16, &Column)> {
    (FIRST_DATE..).zip(dates)
}

/// The first column after the dates.
fn totals_start(timesheet: &Timesheet) -> u16 {
    FIRST_DATE + timesheet.dates.len() as u16
}

/// A format, on grey when its column is owed nothing.
fn shaded(format: &Format, column: &Column) -> Format {
    if column.hours.is_zero() { filled(format, REST) } else { format.clone() }
}

fn filled(format: &Format, colour: u32) -> Format {
    format.clone().set_pattern(FormatPattern::Solid).set_background_color(Color::RGB(colour))
}

/// Writes the people's names and departments.
fn people(sheet: &mut Worksheet, rows: &[Row]) -> Result<(), XlsxError> {
    for (row, person) in (FIRST_PERSON..).zip(rows) {
        sheet.write_string(row, 0, &person.member.display_name)?;
        if let Some(department) = &person.member.department {
            sheet.write_string(row, 1, department)?;
        }
    }
    Ok(())
}

/// Writes lines of explanation under the grid, one per row.
fn legend(sheet: &mut Worksheet, timesheet: &Timesheet, lines: &[String]) -> Result<(), XlsxError> {
    let faint = Format::new().set_font_color(Color::RGB(INK_FAINT));
    let first = FIRST_PERSON + timesheet.rows.len() as u32 + 1;
    for (row, line) in (first..).zip(lines) {
        sheet.write_string_with_format(row, 0, line, &faint)?;
    }
    Ok(())
}

/// The calendar's exceptions in the month, as one line - or none when the
/// month has none.
fn calendar_line(timesheet: &Timesheet) -> Option<String> {
    let days: Vec<String> = timesheet
        .dates
        .iter()
        .filter_map(|column| {
            let kind = match column.calendar? {
                CalendarDayKind::Holiday => "holiday",
                CalendarDayKind::ShortDay => "short day",
                CalendarDayKind::WorkingWeekend => "working weekend",
            };
            let date = column.date.format("%-d %b");
            Some(match &column.note {
                Some(note) => format!("{date} {kind} ({note})"),
                None => format!("{date} {kind}"),
            })
        })
        .collect();
    (!days.is_empty()).then(|| format!("Calendar: {}.", days.join("; ")))
}

/// Hours as a person writes them: `8`, `7.5`.
fn hours_label(hours: Decimal) -> String {
    hours.normalize().to_string()
}

fn hours(seconds: i64) -> f64 {
    seconds as f64 / SECONDS_PER_HOUR
}

// The hours -------------------------------------------------------------------

const HOURS_TOTALS: Totals = Totals {
    headers: &["Days worked", "Worked (h)", "Due (h)", "Norm (h)", "Vacation", "Sick", "Day off", "No record"],
};

/// The letter a day away is filed under.
fn code(kind: WorkdayKind) -> &'static str {
    match kind {
        WorkdayKind::Vacation => "V",
        WorkdayKind::Sick => "S",
        WorkdayKind::DayOff => "O",
        // Not away; never asked. A letter rather than a panic, since a file
        // with one odd cell is better than no file.
        WorkdayKind::Work => "W",
    }
}

fn hours_sheet(timesheet: &Timesheet, month: &str) -> Result<Worksheet, XlsxError> {
    let mut sheet = Worksheet::new();
    sheet.set_name("Timesheet")?;
    frame(&mut sheet, timesheet, &format!("Timesheet - {month}"), &HOURS_TOTALS)?;
    people(&mut sheet, &timesheet.rows)?;

    let centred = Format::new().set_align(FormatAlign::Center);
    // Two places, which is what payroll reads; the value underneath is exact,
    // so a column of them sums to the total beside it.
    let figure = centred.clone().set_num_format("0.00");
    let open = centred.clone().set_italic().set_font_color(Color::RGB(INK_FAINT));
    let unrecorded = filled(&centred.clone().set_font_color(Color::RGB(INK_WARN)), ATTENTION);
    let total = Format::new().set_num_format("0.00");

    for (row, person) in (FIRST_PERSON..).zip(&timesheet.rows) {
        for ((col, column), cell) in date_columns(&timesheet.dates).zip(&person.days) {
            match cell.state {
                CellState::Worked => {
                    sheet.write_number_with_format(row, col, hours(cell.worked_seconds.unwrap_or(0)), &shaded(&figure, column))?;
                }
                CellState::Open => {
                    sheet.write_string_with_format(row, col, "…", &shaded(&open, column))?;
                }
                CellState::Away => {
                    sheet.write_string_with_format(row, col, code(cell.kind.unwrap_or_default()), &shaded(&centred, column))?;
                }
                CellState::Unrecorded => {
                    sheet.write_string_with_format(row, col, "?", &unrecorded)?;
                }
                CellState::Ahead | CellState::Free => {
                    sheet.write_blank(row, col, &shaded(&centred, column))?;
                }
            }
        }

        let member = &person.member;
        let counts = &person.counts;
        let col = totals_start(timesheet);
        sheet.write_number(row, col, member.days_recorded as f64)?;
        sheet.write_number_with_format(row, col + 1, hours(member.worked_seconds), &total)?;
        sheet.write_number_with_format(row, col + 2, hours(member.due_seconds), &total)?;
        sheet.write_number_with_format(row, col + 3, hours(member.norm_seconds), &total)?;
        sheet.write_number(row, col + 4, counts.vacation as f64)?;
        sheet.write_number(row, col + 5, counts.sick as f64)?;
        sheet.write_number(row, col + 6, counts.day_off as f64)?;
        sheet.write_number(row, col + 7, counts.unrecorded as f64)?;
    }

    let mut lines = vec![
        "Hours are worked time: each day's span less its pauses.".to_string(),
        "V vacation, S sick leave, O day off, ? owed and nothing recorded, … a day still open.".to_string(),
        "Grey columns owe nothing: weekends and holidays. Full day (h) is what a date asks of someone full time; Norm (h) is each person's share of it, less their days away.".to_string(),
        format!("Due (h) is the part of the norm that had come due on {}: the dates before it, and that day once its day was closed.", timesheet.today),
    ];
    lines.extend(calendar_line(timesheet));
    legend(&mut sheet, timesheet, &lines)?;
    Ok(sheet)
}

// The reports -----------------------------------------------------------------

const REPORT_TOTALS: Totals = Totals {
    headers: &["On time", "Late", "Missing", "Pending", "Approved"],
};

fn reports_sheet(timesheet: &Timesheet, month: &str) -> Result<Worksheet, XlsxError> {
    let mut sheet = Worksheet::new();
    sheet.set_name("Reports")?;
    frame(&mut sheet, timesheet, &format!("Reports - {month}"), &REPORT_TOTALS)?;
    people(&mut sheet, &timesheet.rows)?;

    let centred = Format::new().set_align(FormatAlign::Center);

    for (row, person) in (FIRST_PERSON..).zip(&timesheet.rows) {
        for ((col, column), cell) in date_columns(&timesheet.dates).zip(&person.days) {
            let Some(filed) = cell.filed else {
                sheet.write_blank(row, col, &shaded(&centred, column))?;
                continue;
            };
            let (mark, ink) = match filed {
                Filed::OnTime => ("✓", INK_OK),
                Filed::Late => ("L", INK_WARN),
                Filed::Missing => ("✗", INK_BAD),
                Filed::Pending => ("·", INK_FAINT),
            };
            // The mark says when the day was reported; the fill says what a
            // manager made of it.
            let base = centred.clone().set_font_color(Color::RGB(ink));
            let format = match cell.report {
                Some(Status::Approved) => filled(&base, APPROVED),
                Some(Status::Returned | Status::Changed) => filled(&base, ATTENTION),
                Some(Status::Submitted) | None => shaded(&base, column),
            };
            sheet.write_string_with_format(row, col, mark, &format)?;
        }

        let counts = &person.counts;
        let col = totals_start(timesheet);
        for (offset, count) in (0u16..).zip([counts.on_time, counts.late, counts.missing, counts.pending, counts.approved]) {
            sheet.write_number(row, col + offset, count as f64)?;
        }
    }

    let mut lines = vec![
        "A day's report is on time when it arrives before the person's next working day begins.".to_string(),
        "✓ on time, L late, ✗ not reported - or nothing recorded on a date owed - and · not due yet.".to_string(),
    ];
    lines.push(if timesheet.day_approval {
        "Green: approved, and the day still comes to what was approved. Orange: returned, or changed since it was reported.".to_string()
    } else {
        "This server does not ask for days to be approved. Orange: returned, or changed since it was reported.".to_string()
    });
    legend(&mut sheet, timesheet, &lines)?;
    Ok(sheet)
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;
    use crate::{
        team::Member,
        timesheet::{Cell, Counts},
    };

    fn date(text: &str) -> NaiveDate {
        text.parse().expect("a test date")
    }

    fn column(text: &str, hours: i64, calendar: Option<CalendarDayKind>) -> Column {
        Column {
            date: date(text),
            hours: Decimal::from(hours),
            calendar,
            note: calendar.map(|_| "Unity Day".to_string()),
            weekend: false,
        }
    }

    fn timesheet() -> Timesheet {
        let cells = vec![
            Cell {
                date: date("2026-11-02"),
                state: CellState::Worked,
                kind: Some(WorkdayKind::Work),
                worked_seconds: Some(27_000),
                norm_seconds: 28_800,
                filed: Some(Filed::Late),
                report: Some(Status::Approved),
            },
            Cell {
                date: date("2026-11-03"),
                state: CellState::Away,
                kind: Some(WorkdayKind::Sick),
                worked_seconds: Some(0),
                norm_seconds: 0,
                filed: None,
                report: None,
            },
            Cell {
                date: date("2026-11-04"),
                state: CellState::Free,
                kind: None,
                worked_seconds: None,
                norm_seconds: 0,
                filed: None,
                report: None,
            },
        ];
        Timesheet {
            month: "2026-11".into(),
            from: date("2026-11-01"),
            to: date("2026-11-30"),
            today: date("2026-11-20"),
            standard_hours: Decimal::from(8),
            day_approval: true,
            dates: vec![
                column("2026-11-02", 8, None),
                column("2026-11-03", 8, None),
                column("2026-11-04", 0, Some(CalendarDayKind::Holiday)),
            ],
            rows: vec![Row {
                member: Member {
                    id: uuid::Uuid::nil(),
                    display_name: "Ada".into(),
                    email: "ada@example.test".into(),
                    department: None,
                    department_id: None,
                    active: true,
                    days_recorded: 1,
                    worked_seconds: 27_000,
                    paused_seconds: 0,
                    last_day: None,
                    day_open: false,
                    last_seen_at: None,
                    agents: 0,
                    work_rate: Decimal::ONE,
                    norm_seconds: 28_800,
                    due_seconds: 28_800,
                    days_away: 1,
                },
                counts: Counts::of(&cells),
                days: cells,
            }],
        }
    }

    #[test]
    fn a_month_writes_as_a_workbook() {
        let bytes = write(&timesheet()).expect("the workbook should write");
        // A zip's local file header. What the sheets say is read back by the
        // integration tests, through a real unzip.
        assert_eq!(&bytes[..4], b"PK\x03\x04");
    }

    #[test]
    fn every_letter_names_one_kind_of_day_away() {
        let letters: Vec<&str> = [WorkdayKind::Vacation, WorkdayKind::Sick, WorkdayKind::DayOff].into_iter().map(code).collect();
        assert_eq!(letters, ["V", "S", "O"]);
    }

    #[test]
    fn the_calendar_line_names_the_months_exceptions() {
        assert_eq!(calendar_line(&timesheet()).as_deref(), Some("Calendar: 4 Nov holiday (Unity Day)."));

        let mut plain = timesheet();
        plain.dates.retain(|column| column.calendar.is_none());
        assert_eq!(calendar_line(&plain), None, "a month with no exceptions says nothing about them");
    }

    #[test]
    fn hours_are_written_the_way_a_person_writes_them() {
        assert_eq!(hours_label(Decimal::new(80, 1)), "8");
        assert_eq!(hours_label(Decimal::new(75, 1)), "7.5");
    }
}
