# 0024. The timesheet

Date: 2026-10-15

Status: Accepted

## Context

At the end of a month somebody has to say what it came to. A timesheet is the
document that says it: a row per person, a column per date, the hours of each
day worked, a letter for each day away, the month's totals - checked by a
manager, filed with payroll or HR, sometimes printed and signed. kasl will file
one person's month this way from their own machine (`kasl timesheet`); the
server holds everybody's, and until now could only hand them out as a range
export, a long table a person cannot check a month against.

The pieces were already here. The production calendar and the norm say what
each date asked of each person (ADR 0017), and the part of it that has come due
(ADR 0023). Leave is a kind of day (ADR 0017). Reports say who put their name
to which day, and approvals what a manager made of it (ADR 0022). What was
missing was the month assembled from them - and four decisions about it.

**Who decides what a date was.** The heatmap answered only the dates with a
workday and left everything else to the screen (ADR 0015). A timesheet cannot:
"owed, come due and not recorded" is the cell a manager is checking a month
for, and deciding it needs the calendar, the person's rate, their leave and the
server's date - the norm's rule. A screen that worked it out again would be a
second copy of that rule, free to disagree with the file the same manager
downloads.

**Whose month.** The team table, the summary export and the heatmap each
listed people their own way: the first two everyone active plus anyone with
days in the range (ADR 0023), the heatmap the active only. A timesheet that
dropped somebody who worked in the month and has left since would be wrong in
exactly the place a timesheet is used - their last payroll.

**When a report is late.** The approval milestone left "sent late" and the
✓/L/✗ matrix of the approved mockup for this one, because they need a
deadline, and a deadline needs a moment. "By midnight" needs the person's time
zone, which the server does not store (ADR 0003). "Within N hours of the day's
end" is a setting, and a setting an operator would have to invent rather than
record.

**What the file is.** The range export's workbook is two long tables. A
timesheet is a grid, and a grid of 365 columns is not a timesheet - so it is
not a third sheet of the range export.

## Decision

**A month, answered date by date.** `GET /api/v1/team/timesheet?month=YYYY-MM`
for everyone the reader may see, `GET /api/v1/me/timesheet?month=YYYY-MM` for
the reader alone. Every date of the month is a cell for every person, and the
server says what it was: `worked`, `open`, `away`, `unrecorded`, `ahead` or
`free`, with the hours, the kind of day and what the date asked of the person.
A date owed, come due and holding nothing is `unrecorded` - never a zero. The
month's dates are answered beside the rows with what each is worth at a full
day and what the calendar calls it.

**A row is the summary's row.** The people and their figures come from the
function behind `/team/days` and the summary export, flattened into the row
unchanged, so the three cannot disagree about anybody's month - including who
is on it. The cells add up to the row: their norms to its norm, their hours to
its hours.

**A report is on time when it arrives before the person's next working day
begins.** The deadline is a moment the agent already reported: when the person
started their next day of work - the next date after this one with a day of
kind `work`, however far ahead. Reported before it, or with none begun yet: on
time. After it: late. Not reported, and it has begun: missing; not begun yet:
pending - there is still time. A date owed and gone by with nothing recorded is
missing too: there was nothing to put a name to, and that is the finding. It
needs no time zone and no setting, and it is the rhythm kasl already has: a day
is reported when it is closed, before another one starts. Leave is not a
working day, so a Friday reported during the week away is on time.

Timeliness is the **first** report's, status the **newest**'s. A day reported
in time and corrected later was reported in time; a late day reported again is
still late. Where the day stands now - approved, returned, changed - stays the
newest report's, derived as ADR 0022 derives it.

**The file is a grid, by the month.** `/team/timesheet.xlsx` and
`/me/timesheet.xlsx` answer the same month as a workbook of two sheets over one
frame: **Timesheet** (hours, letters for leave, `?` for a date owed and
unrecorded, the totals) and **Reports** (✓ L ✗ ·, filled by what a manager made
of each). The names and dates stay in view, the dates repeat on every printed
page, and the sheet prints landscape, one page wide. Downloading other people's
month is audited as `hours.exported` with `"file": "timesheet"`, the same read
as ADR 0023's exports.

**One answer behind the month screen.** The web UI's month screen draws three
views of the one timesheet answer - the timesheet, the reports, and the shape
(the heatmap) - so switching between them is instant and they cannot disagree.
A sixth tab on the phone's bar would not fit; three views of one month are one
screen.

## Consequences

`GET /api/v1/team/heatmap` keeps its shape but is now built from the
timesheet: its people are the timesheet's - somebody who left stays on the
months they worked, which reverses ADR 0015's "a deactivated person leaves the
grid" for months they have days in - its figures come from `workday_figures`
rather than its own SQL, and each cell carries its `kind`, so a day of sick
leave is no longer a square of zero hours shaded as the shortest day worked.
The busiest day is a day of work.

The demo's reports were written at the moment of the seed, which under this
deadline made every one of them late. They are backdated to minutes after their
day ended; one person files two days the next morning and another forgets one,
so all three marks are on the demo. One person takes three days of leave, the
week before last - not last week, where the signals would read it as an unusual
week: the signals do not know about leave yet, which is recorded as their next
step rather than fixed here. The demo's agents now follow its calendar, resting
on the holiday and working the transferred Saturday; before, the timesheet drew
the whole team owing a Saturday nobody had been told to work.

Planned leave - a vacation booked for next month - has no place here: leave is
a kind of day the agent reports, so it is known once it is lived. A booked
absence is its own record, and its own milestone.
