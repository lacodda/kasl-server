---
title: Exporting hours
description: The team's hours, or your own, over any range - as an Excel workbook or as CSV files.
---

A range of hours leaves the server as two tables. **Summary** has a row per
person, with the figures the team screen shows for the same range. **Days** has
a row per person and date. The workbook holds both tables as sheets; each CSV
file holds one table.

| File | Who may download it |
| --- | --- |
| `GET /api/v1/team/export.xlsx` | managers and administrators: everyone they may see |
| `GET /api/v1/team/export/summary.csv` | the same |
| `GET /api/v1/team/export/days.csv` | the same |
| `GET /api/v1/me/export.xlsx` | anyone signed in: their own hours |
| `GET /api/v1/me/export/summary.csv` | the same |
| `GET /api/v1/me/export/days.csv` | the same |

All six take `from` and `to`, both inclusive, at most 400 days apart, the same
as [`/team/days`](/kasl-server/reference/the-teams-hours/):

```console
$ curl -OJ -H "Cookie: kasl_session=..." \
    "http://127.0.0.1:8080/api/v1/team/export/summary.csv?from=2026-09-01&to=2026-09-30"

$ cat kasl-team-2026-09-01-to-2026-09-30-summary.csv
person,email,department,active,work_rate,days_worked,days_away,worked_hours,paused_hours,norm_hours
Anna,anna@example.com,Engineering,true,1,20,2,158.25,19.50,160.00
Clara,clara@example.com,,true,0.5,10,0,41.00,3.75,88.00
```

The file name says whose hours and which range, and the answer carries
`Cache-Control: no-store`: these are people's working hours, and a shared
machine's cache is no place to keep a copy of them.

## The summary

One row per person, in the team screen's order, read by the same code that
answers `/team/days` - a spreadsheet and the screen it was downloaded from
cannot disagree. Everyone the reader may see is listed, including people with
nothing recorded. Somebody deactivated since is listed for a range they have
days in, with `active` false: August's total does not shrink because somebody
who worked in August left in October.

| CSV column | Sheet column | What it is |
| --- | --- | --- |
| `person` | Person | display name |
| `email` | Email | |
| `department` | Department | empty for a person in no department |
| `active` | Active | `false` for an account deactivated since |
| `work_rate` | Share of a full day | `0.5` is half time |
| `days_worked` | Days worked | days recorded as work |
| `days_away` | Days away | days recorded as leave, illness or a day off |
| `worked_hours` | Worked (h) | finished days only - an open day has no total yet |
| `paused_hours` | Paused (h) | |
| `norm_hours` | Norm (h) | what the range asked of this person, leave excused - see [the calendar and the norm](/kasl-server/reference/the-calendar-and-the-norm/) |

## The days

One row per person and date: **every date that was recorded, and every date
that was due without being recorded.** A due date with no record has its norm
and nothing else - absent data stays absent, it is not written down as zero
hours worked. A date that was neither recorded nor due, such as a weekend nobody
worked, is left out.

Both kinds are there so that **a person's days add up to their summary row**:
the worked hours, the paused hours and the norm alike. A reader reconciling the
two tables finds nothing missing.

| CSV column | Sheet column | What it is |
| --- | --- | --- |
| `person`, `email`, `department` | | as in the summary |
| `date` | Date | the employee's own calendar date |
| `kind` | Kind | `work`, `vacation`, `sick`, `day_off`; empty for a date with no record |
| `started_at` | Started (UTC) | |
| `ended_at` | Ended (UTC) | empty while the day is open |
| `worked_hours` | Worked (h) | empty while the day is open |
| `paused_hours` | Paused (h) | |
| `norm_hours` | Norm (h) | the date's norm; `0` on leave, a weekend or a holiday |
| `report` | Report | the day's [report](/kasl-server/reference/reports-and-approval/): `submitted`, `approved`, `returned`, `changed`; empty if it was never reported |

**Times are UTC.** The server keeps the instant and the employee's own date,
not their time zone, so a start time can only be stated in one zone that means
the same thing for everybody. The `date` column is the employee's.

## CSV and the workbook

**CSV is for programs.** Headers are the `snake_case` names above, fields are
comma-separated and quoted only where they have to be, lines end in CRLF, the
encoding is UTF-8 with no byte-order mark. Hours are written to two decimal
places. Instants are ISO 8601 (`2026-09-14T12:03:00Z`), dates `2026-09-14`.

A name or a department that starts with `=`, `+`, `-` or `@` is written with an
apostrophe in front of it. A CSV is opened in a spreadsheet as often as it is
parsed, and text that starts that way runs there as a formula. Numbers are never
changed.

**The workbook is for people.** The two sheets have headers in words, the header
row stays in view, every column has a filter, dates are dates and hours are
numbers. Each hour figure is stored exactly and shown to two places, so a column
sums to the summary to the second.

## Recorded

Downloading other people's hours is written to
[the audit log](/kasl-server/reference/the-audit-log/) as `hours.exported`,
with the file, the range and how many people it held. It is the one read after
which the data has left the server. Downloading your own hours is not recorded.

The reasoning is in
[ADR 0023](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0023-periods-comparison-and-export.md).
