---
title: The month as a shape
description: GET /api/v1/team/heatmap - the same team a day at a time, so a pattern across weeks becomes visible.
---

`GET /api/v1/team/heatmap?month=YYYY-MM` answers the same team a day at a time:
a row per person, a cell per date they recorded. It is `kasl sum` widened by
one axis - what the week's totals cannot show is the *pattern*, and the pattern
is how a manager notices somebody working weekends or filing nothing since the
12th.

```console
$ curl -H "Cookie: kasl_session=..." "http://127.0.0.1:8080/api/v1/team/heatmap?month=2026-08"
{"month":"2026-08","from":"2026-08-01","to":"2026-08-31","busiest_seconds":34200,
 "rows":[
   {"user_id":"c49ea6a8-...","display_name":"Anna","department":"Engineering",
    "worked_seconds":72600,"busiest_seconds":34200,
    "days":[{"date":"2026-08-24","worked_seconds":27000,"open":false},
            {"date":"2026-08-25","worked_seconds":34200,"open":false},
            {"date":"2026-08-26","worked_seconds":null,"open":true}]},
   {"user_id":"0073460d-...","display_name":"Clara","department":null,
    "worked_seconds":0,"busiest_seconds":null,"days":[]}]}
```

**A date that is not in the row was not recorded.** The server does not
manufacture a zero for it. Clara's empty `days` and a day of no hours are
different facts, and a grid that painted them the same shade would say an
employee who never installed kasl took the month off - the reading a manager
reaches for first, and the false one ([ADR 0011](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0011-the-privacy-manifest.md)). The web UI draws the absence
as an empty square and says so in the legend.

**An open day carries `worked_seconds: null`**, exactly as `/me/days` answers
it. A day half-lived is not a short day.

**The scale is the data's own.** The response carries seconds and each row's
busiest day, plus the busiest anywhere in the grid; how those become colour is
the screen's business. A threshold shipped in the API would be this server
asserting what a normal working day is, and it has no opinion on that until
norms arrive with the production calendar
([ADR 0015](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0015-the-month-heatmap.md)).

`month` must be `YYYY-MM`. A date, a bare year, or a missing parameter is a
`400` rather than a guess - a caller who sent one would otherwise get a
plausible answer to a question they did not ask. Visibility is the same rule as
every other team endpoint, and a person with nothing recorded is listed anyway.
