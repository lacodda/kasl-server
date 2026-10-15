---
title: The timesheet
description: GET /api/v1/team/timesheet - every date of everybody's month, what it was, and whether it was reported in time; and the workbook a company files.
---

A timesheet is what a month came to, person by person and date by date, in the
shape a company files it. Every date of the month says what it was - a day
worked and its hours, a day away and why, a day owed and never recorded, a day
still to come - with the month's totals beside it. It is the team's half of
`kasl timesheet`, which files one person's month from their own machine.

| Route | Who may read it |
| --- | --- |
| `GET /api/v1/team/timesheet?month=YYYY-MM` | managers and administrators: everyone they may see |
| `GET /api/v1/me/timesheet?month=YYYY-MM` | anyone signed in: their own month, one row |
| `GET /api/v1/team/timesheet.xlsx?month=YYYY-MM` | as `/team/timesheet`, as a workbook |
| `GET /api/v1/me/timesheet.xlsx?month=YYYY-MM` | as `/me/timesheet`, as a workbook |

```console
$ curl -H "Cookie: kasl_session=..." "http://127.0.0.1:8080/api/v1/team/timesheet?month=2026-08"
{"month":"2026-08","from":"2026-08-01","to":"2026-08-31","today":"2026-10-15",
 "standard_hours":8.0,"day_approval":true,
 "dates":[{"date":"2026-08-01","hours":0.0,"calendar":null,"note":null,"weekend":true},
          ...
          {"date":"2026-08-31","hours":0.0,"calendar":"holiday","note":"Summer holiday","weekend":false}],
 "rows":[
   {"id":"c49ea6a8-...","display_name":"Anna","department":"Engineering","active":true,
    "work_rate":1.0,"days_recorded":6,"worked_seconds":153000,"norm_seconds":547200,
    "due_seconds":547200,"days_away":1, ...,
    "days":[{"date":"2026-08-03","state":"worked","kind":"work","worked_seconds":30600,
             "norm_seconds":28800,"filed":"on_time","report":"approved"},
            {"date":"2026-08-06","state":"away","kind":"vacation","worked_seconds":0,
             "norm_seconds":0,"filed":null,"report":null},
            {"date":"2026-08-07","state":"unrecorded","kind":null,"worked_seconds":null,
             "norm_seconds":28800,"filed":"missing","report":null},
            ...],
    "counts":{"vacation":1,"sick":0,"day_off":0,"unrecorded":14,
              "on_time":2,"late":1,"missing":15,"pending":1,"approved":1}}]}
```

## A row is the summary's row

The people and their totals are the ones [`/team/days`](/kasl-server/reference/the-teams-hours/)
answers for the same month, field for field - the same function builds both - so
a timesheet cannot disagree with the team screen or with
[the summary export](/kasl-server/reference/exporting-hours/) about anybody.
That includes who is on it: everyone active the reader may see, including people
with nothing recorded, and anyone deactivated since who has days in the month.
A month is a record of what happened in it, and August's total must not shrink
in October because somebody who worked in August has left.

A row's cells add up to it: the `norm_seconds` of its dates sum to the row's
`norm_seconds`, and their `worked_seconds` to its `worked_seconds`.

## What a date was

Every date of the month is a cell, and its `state` is decided by the server -
which dates were owed and which have come due is the
[norm's](/kasl-server/reference/the-calendar-and-the-norm/) rule, and a screen
that worked it out again would be a second copy of it.

| `state` | What it means |
| --- | --- |
| `worked` | a finished day of work, with `worked_seconds` |
| `open` | a day of work still running on the agent; no total yet |
| `away` | a day the person was on leave, ill or off - `kind` says which; owes nothing |
| `unrecorded` | owed, come due, and nothing recorded |
| `ahead` | owed, and not come due yet: a date to come, or today before its day is in |
| `free` | owed nothing and nothing recorded: a weekend, a holiday |

A date that was owed, has come due and holds nothing is `unrecorded`, with no
hours - never a zero. A day of zero hours and a day nobody recorded are
different facts.

`norm_seconds` is what the date asked of this person: the calendar's hours at
their share of a full day, and nothing on a day they were away. `dates` carries
the calendar itself - what each date is worth at a full day (`hours`), and what
the calendar calls it.

## Reported in time

**A day's report is on time when it arrives before the person's next working
day begins.** The deadline is the moment their next day of work started - the
next date after it with a day of `kind` `work` - so it needs no setting and no
time zone, and it is the rhythm kasl already has: a day is reported when it is
closed, before another one starts.

| `filed` | When |
| --- | --- |
| `on_time` | the first report arrived before the next working day began, or none has begun yet |
| `late` | the first report arrived after it began |
| `missing` | no report, and the next working day has begun; or a date `unrecorded` |
| `pending` | no report yet, and the next working day has not begun - there is still time |

A Friday reported on Saturday is on time; reported on Monday after work began,
it is late. Leave between the two does not move the deadline: a day reported
during a week away is on time, because a week away is not a working day.

Timeliness is the **first** report's. A day reported in time and corrected
later was still reported in time, and a late day reported again stays late.
Where the day stands now - approved, returned, changed since - is the
**newest** report's, in `report`, the same status the personal page shows
([reports and approval](/kasl-server/reference/reports-and-approval/)).

Only a finished day of work asks for a report, and a date owed and gone by
with nothing recorded counts as one not reported. A day away, an open day, a
date to come and a date owed nothing have `filed: null`.

## The workbook

The `.xlsx` routes answer the same month as a workbook for people to check,
print and sign. Two sheets over one frame: the dates as columns with their
weekdays, what each is worth at a full day, and grey columns where nothing is
owed.

- **Timesheet** - the hours of each day worked (exact, shown to two places), a
  letter for each day away (`V` vacation, `S` sick leave, `O` day off), `?` for
  a date owed and unrecorded, `…` for a day still open, and the totals: days
  worked, worked, due and norm hours, and the counts of each kind.
- **Reports** - `✓` on time, `L` late, `✗` not reported, `·` not due yet; a
  green cell is approved, an orange one returned or changed since. The totals
  count each.

The names and the dates stay in view as the sheet scrolls, and on paper the
dates repeat on every page. The file is named `kasl-team-timesheet-2026-08.xlsx`
(or `kasl-me-...`) and carries `Cache-Control: no-store`. Downloading other
people's month is written to [the audit log](/kasl-server/reference/the-audit-log/)
as `hours.exported` with `"file": "timesheet"`; downloading your own is not.

A program wanting a row per person and date reads the
[days CSV](/kasl-server/reference/exporting-hours/) instead: the timesheet is a
grid because a grid is what a person checks a month against.

`month` must be `YYYY-MM`; anything else is a `400`. The reasoning is in
[ADR 0024](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0024-the-timesheet.md).
