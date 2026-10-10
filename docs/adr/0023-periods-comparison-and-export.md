# 0023. Periods, comparison and export

Date: 2026-10-10

Status: Accepted

## Context

Every screen with hours on it showed a week. A manager asked "how did
September go" had to page through four and a half of them and add up in their
head; an employee asked "am I on track this month" had the same arithmetic to
do. And the hours could not leave: a payroll run, an invoice to a client or a
quarterly review all start from a spreadsheet, and the server that holds the
hours had no way to give one.

Three things were settled before any of it could be built.

**The figures must be the screen's.** A month exported to a spreadsheet and the
same month on the team screen are one claim; if they disagree by a second, the
spreadsheet is the one people believe and the screen the one they stop
trusting. Two pieces of arithmetic stood in the way. The team table computed a
day's worked seconds with its own copy of the rule `workday_figures` holds
(ADR 0022), and that copy rounded where the view rounds down - so a person's
week and the sum of its days disagreed by up to a second a day. And the paused
seconds the view subtracts were not a column of the view, so they were derived
again beside it.

**A period is a record of what happened in it.** The team table listed active
people only. Paging back to August after somebody left in September made their
August hours vanish from August's total - the same period answering differently
depending on when it was asked. For a screen about this week that went
unnoticed; for a month exported to payroll it is a defect.

**This product does not rank people.** ADR 0016 says so in as many words: a
dashboard that orders people by their hours is a scoreboard. "Compare
employees" therefore cannot mean "sort by hours", where a full-timer always
beats somebody on half time and somebody back from holiday always comes last.

## Decision

**A period is a day, a week or a month**, and the team screen and the personal
page both show any of them. Weeks run Monday to Sunday, months are calendar
months. The API needed no new route for this: `/team/days` and `/me/days` have
taken a range from the first version, and a period is a range. The unit lives
in the client and in its URL (`?period=month&from=2026-09-01`), so a period can
be linked and survives a visit to a person's page and back.

**People are compared with their own norm, and with themselves.** The team
table sorts by name, by the share of their norm a person worked, or by how that
share moved against the period before. The share of norm is the one figure that
is comparable between people without being a scoreboard: it is already
adjusted for a part-time rate and for leave (ADR 0017), so somebody on half
time who worked their half reads as 100%, the same as a full-timer who worked a
full week. There is deliberately no sort by hours.

**Periods are compared with the period before, of the same unit**: this week
with last week, September with August, a day with the day before. The figure
compared is the share of norm, for the same reason - September has more working
days than August, and comparing raw hours across them compares calendars. Where
either period owed nothing (a Sunday, a fortnight of leave) there is no share
and no comparison, rather than an infinite one. The comparison is the client's
arithmetic on two answers of the same endpoint; the server answers pairs and
the screen divides (ADR 0017).

**A period still running is measured against the norm that has come due.**
On the 10th, the month's whole norm makes somebody exactly on track read as a
third done; the norm "up to today" is still wrong in the morning, because it
counts today while today's day is open, and an open day has no total. So the
server answers `due_seconds` beside `norm_seconds` - on every row of
`/team/days` and in the `progress` of `/me/days`: every date before today, and
today once its day is closed. For a period that is over the two are equal. The
share of norm is worked over due, so a share compares the same thing whether
the period is over or not, and a period not yet begun has nothing due and no
share. "Today" is the server's date - the approximation "is a day open" already
makes, from the same clock (ADR 0003). The screens show the due norm, with the
whole period's beside it while they differ.

**One function answers the team's rows**, for the table and for the export
alike, and it reads `workday_figures`, which now carries `paused_seconds`
beside `worked_seconds`. The team total is the sum of the days to the second.

**Somebody deactivated is listed for a period they have days in**, marked
`active: false`, and not for a period without them. Today's week still lists the
people who are here today.

**Hours leave as two tables.** The summary has a row per person - the team
table's rows, read by the same function. The days table has a row per person
and date, and holds every date that was recorded *and every date that was due
without being recorded*, with its norm and nothing else. With both, a person's
days add up to their summary row, worked and norm alike; without the due dates,
the norm column of the days would sum to less than the summary's and a reader
reconciling them would find a hole that is not there. A date that was neither
recorded nor due is left out.

**CSV is for programs, the workbook for people.** Each CSV holds one table,
with `snake_case` headers, comma-separated, CRLF, UTF-8 without a byte-order
mark, hours to two decimal places. The workbook holds both tables as sheets,
with words for headers, dates as dates and exact hour figures shown to two
places. Both are rendered from one list of columns. The files are named as files
- `/team/export.xlsx`, `/team/export/summary.csv`, `/team/export/days.csv`, and
the same under `/me` - so a browser saves them as what they are.

**Text that a spreadsheet would run is defused in CSV.** A name or a department
that begins with `=`, `+`, `-` or `@` gets an apostrophe in front of it (the
OWASP mitigation for CSV injection). Numbers are never changed. The workbook
needs nothing of the kind: its strings are written as strings.

**Exporting other people's hours is audited.** `hours.exported` records the
file, the range and how many people it held. Reads are not otherwise recorded
(ADR 0010); this one is, because after it the data has left the server.
Exporting your own hours is not recorded.

## Consequences

- The team screen answers "how did September go" in one view, the personal page
  answers "am I on track this month", and both can be handed to a spreadsheet.
- An export is of the dates asked for. The screens ask for the period up to
  today while it runs, so a file downloaded on the 10th holds the month so far
  and no dates that are due but have not come yet.
- The norm of somebody deactivated covers the whole range: the server does not
  record when an account was deactivated, so it cannot stop asking on that day.
  Their hours are right; their norm in a month they left is overstated.
- A day's start and end leave in UTC. The server keeps the instant and the
  employee's own date but not their zone (ADR 0003), and a time stated in the
  reader's zone would be a different time for every reader of the same file.
- The workbook is built in memory. A team of two hundred over a year is about
  fifty thousand day rows, a few megabytes; streaming it would be a different
  writer for a size nobody has.
- `rust_xlsxwriter` is a new dependency. Writing the format by hand means a zip
  writer, shared strings, styles and the 1900 date system; the crate does all of
  that and nothing else, with no default features.
