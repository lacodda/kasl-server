# 0015. The month heatmap

Date: 2026-09-01

Status: Accepted

## Context

The dashboard answers two questions well: what the team did over one week, and
what each person is doing right now. Neither shows a *shape*. A manager who
wants to know who works weekends, whose month is ragged, or who has quietly
stopped filing days after the 12th has to page through weeks one at a time and
hold the picture in their head.

`kasl sum` answers exactly this on one machine - the employee's month, day by
day. The server has the same rows for everyone, and the missing screen is that
summary widened by one axis.

`/team/days` cannot be that screen. It answers a period as one row per person:
totals, a last date, a bar. Adding a per-day breakdown to it would change the
cost of the query the dashboard runs on every page load, for numbers that
screen does not draw - the same argument that put the pulse on its own route
(ADR 0014).

## Decision

**A cell is a person and a local date, and it may be empty.** `GET
/api/v1/team/heatmap?month=YYYY-MM` answers one row per visible person and,
inside it, only the dates that have a workday. A date absent from the row is
absent from the data: the server does not manufacture a zero for it.

This is the same rule the privacy work settled at ingest and the dashboard
repeats in words (ADR 0011): nothing recorded and nothing worked look alike,
and only one of them says something about a person. A grid of zeroes would
make an employee who never installed kasl indistinguishable from one who took
the month off, and the reading a manager reaches for first - "they did no
work" - is the false one.

**An open day carries no total.** `worked_seconds` is `null` while a day is
still running, exactly as `/me/days` answers it. A day half-lived is not a
short day, and the cell says "in progress" rather than painting the lightest
shade on the scale.

**The scale is the data's own, and the server does not choose it.** The
response carries the seconds; how they become colour is the screen's business.
A server that shipped thresholds would be shipping a policy about what a
normal day is - and this installation has no norms yet (they arrive with the
production calendar, v0.21). What the response does carry is each row's own
maximum and total, so the screen does not have to re-derive figures the
database already summed.

**A month, bounded by the month.** The parameter is `YYYY-MM` rather than a
free range. The screen pages by month; a range wide enough to be interesting
is wide enough to be expensive, and `/me/days` already exists for anyone who
wants an arbitrary span. The month is resolved against the calendar, not
against a 30-day window, so February is 28 cells and the boundary is never
off by a day.

**Weekends are the reader's arithmetic, not the server's.** A date is a label
(ADR 0003); which of them are working days depends on a calendar this server
does not have until v0.21. The screen marks Saturday and Sunday from the date
itself and says nothing stronger, because "day off" is a claim and "weekend"
is a fact about the calendar.

## Consequences

The heatmap is one query per request, grouped in the database rather than
assembled in Rust: the alternative is fetching every workday of the month for
the whole team and folding it here, which is the same rows over the wire for
no gain.

`VISIBLE_USERS` is pasted in as it is everywhere else, so a manager sees their
departments and an administrator everyone. A person with nothing recorded is
still listed - an empty row is the answer a manager most needs to see, and
dropping them would hide precisely the case the screen was built for.

Thresholds living on the client means the two ends could disagree about what
"a full day" looks like if a second client ever draws this. That is accepted:
until norms exist, any number the server picked would be an invention, and an
invented threshold shipped in an API is harder to take back than one in a
stylesheet.
