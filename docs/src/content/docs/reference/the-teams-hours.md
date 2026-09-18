---
title: The team's hours
description: GET /api/v1/team/days - hours, days recorded and pauses for every person a manager may see.
---

`GET /api/v1/team/days` answers a row per person over a range: hours, days
recorded, pauses, and what the server knows about them right now. Managers and
administrators only.

```console
$ curl -H "Cookie: kasl_session=..."        "http://127.0.0.1:8080/api/v1/team/days?from=2026-08-24&to=2026-08-30"
{"from":"2026-08-24","to":"2026-08-30","privacy_level":"full","not_stored":[],"standard_hours":8,
 "members":[
   {"id":"c49ea6a8-...","display_name":"Anna","email":"anna@example.com","department":"Engineering",
    "days_recorded":3,"worked_seconds":72600,"paused_seconds":8100,"last_day":"2026-08-26",
    "day_open":false,"last_seen_at":"2026-08-26T20:31:00Z","agents":1,
    "work_rate":1,"norm_seconds":144000,"days_away":0},
   {"id":"0073460d-...","display_name":"Clara","email":"clara@example.com","department":null,
    "days_recorded":0,"worked_seconds":0,"paused_seconds":0,"last_day":null,
    "day_open":false,"last_seen_at":null,"agents":0,
    "work_rate":1,"norm_seconds":144000,"days_away":0}]}
```

**Everyone the reader may see is listed, including people with nothing
recorded.** Clara above has no agent installed and no days; she is on the list
anyway, because an employee whose agent never reported is exactly who a manager
needs to notice. A table that dropped her would hide the case it exists for.

**`days_recorded` and `days_away` partition the range's days**: the first
counts days worked, the second days the person told us they were on leave or
ill. Neither includes the other, so "four days, twenty hours" is four days of
work rather than two days of work and two of holiday.

**`norm_seconds` is what the range asked of that person** — the production
calendar at their own `work_rate`, with the days they were on leave taken out
(`days_away` counts them). `standard_hours` is stated once for the table: it is
the installation's full day, and every norm in the rows is computed from it.
See [the calendar and the norm](/kasl-server/reference/the-calendar-and-the-norm/).

A shorter week is not a verdict. A row at half the others' hours with
`work_rate: 0.5` is somebody on half time, and a row short by two days with
`days_away: 2` is somebody who was on holiday — the numbers are there so the
screen can say which.

**`day_open` and `last_seen_at` are about days, not about this moment** -
whether a day is open on that person's own calendar, and when one of their
agents last delivered anything. Who is at their keyboard right now is a
separate question with a separate endpoint - see
[Who is working now](/kasl-server/reference/who-is-working-now/).

**Who sees whom** is the rule departments established: an administrator sees
everyone, a manager sees the departments they run plus themselves, and a person
in no department is visible to the administrator alone
([ADR 0009](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0009-departments-and-visibility.md)).

`GET /api/v1/users/{id}/days` is the drill-down: **the same response as
`/me/days`**, for a person the caller is entitled to see. An id they may not
see answers `404` rather than `403` - a manager probing ids should not be able
to tell an employee in another department from one who does not exist.
