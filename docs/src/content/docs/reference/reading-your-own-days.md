---
title: Reading your own days
description: GET /api/v1/me/days - the signed-in person's own workdays, pauses and tasks.
---

`GET /api/v1/me/days` answers the signed-in person's own history: the workdays
in a range, each with its pauses and the tasks logged on it. Both ends are
inclusive, so one date twice is one day.

```console
$ curl -H "Cookie: kasl_session=..."        "http://127.0.0.1:8080/api/v1/me/days?from=2026-08-24&to=2026-08-30"
{"from":"2026-08-24","to":"2026-08-30","privacy_level":"full","not_stored":[],
 "progress":{"norm_seconds":144000,"standard_hours":8,"work_rate":1},"worked_seconds":29640,
 "days":[{"date":"2026-08-24","kind":"work","norm_seconds":28800,
          "started_at":"2026-08-24T12:05:00Z","ended_at":"2026-08-24T21:12:00Z",
          "worked_seconds":29640,"paused_count":2,"paused_seconds":3180,
          "pauses":[{"id":"56e75449-...","started_at":"2026-08-24T15:30:00Z","ended_at":"2026-08-24T16:15:00Z",
                     "duration_seconds":2700,"manual":true,"reason":"lunch"}],
          "tasks":[{"id":"2a9bd688-...","name":"Read-only endpoint","comment":null,
                    "completeness":100,"recorded_at":"2026-08-24T20:50:00Z"}]}]}
```

`worked_seconds` is the day's span minus what was paused, and it is `null` while
the day is still open — a day in progress has no total, and reporting the hours
so far as the day's figure would make every working afternoon look short.
`paused_count` and `paused_seconds` are always answered, whether they come from
the stored pauses or from the totals a coarse policy keeps instead.

**`progress` is what the range asked for, beside what was worked.** A pair
rather than a percentage: eight hours out of ten and four out of five are the
same percentage and not the same fact, so the server answers both numbers and
the screen divides them. Each day carries its own `norm_seconds` too — zero on
a weekend, a holiday, and a day whose `kind` says the person was away. See
[the calendar and the norm](/kasl-server/reference/the-calendar-and-the-norm/).

**The route is `/me`, not your own id under `/users`.** It consults no role and
no department, so there is no permission here to read wrong; reading someone
else's days arrives with the manager's dashboard as its own route, where the
check is the point. A session is required — an agent's bearer token writes days
and reads the privacy manifest, and that is deliberately the whole list.

**`not_stored` names what the installation's privacy level withholds** —
`pauses`, `tasks`, `free_text`, or nothing at all. It is what lets a screen say
"not stored" where it would otherwise draw an empty section: an employee cannot
tell "no pauses were kept" from "you took no breaks", and only one of those is
true ([ADR 0011](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0011-the-privacy-manifest.md)).

A range covers at most 400 days; a wider one, or one that runs backwards, is a
`400` naming the span asked for rather than a truncated answer.
