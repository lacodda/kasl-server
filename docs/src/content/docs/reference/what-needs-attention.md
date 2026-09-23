---
title: What needs attention
description: GET /api/v1/alerts - a quiet agent, a day that ran past its norm, a day nobody closed.
---

`GET /api/v1/alerts` answers what the server noticed **without being asked**.
Everything else here waits for somebody to open a page. An agent that died on
Monday morning, a day left open across a weekend, somebody's eleventh hour on
a Tuesday — none of those improve by being found on Thursday, and two of them
are worse by then.

```console
$ curl -H "Cookie: kasl_session=..." http://127.0.0.1:8080/api/v1/alerts
{"open":2,"people":12,
 "thresholds":{"alert_silence_hours":12,"alert_overwork_factor":"1.50","alert_open_day_hours":16},
 "alerts":[
   {"id":"7f1c...","user_id":"0073460d-...","display_name":"Jonas Petit","department":"Design",
    "rule":"no_agent_data","state":"open","fired_at":"2026-09-19T06:15:00Z",
    "observed_seconds":690300,"against_seconds":43200,"subject_date":null,
    "resolved_at":null,"acknowledged_at":null,"acknowledged_by":null},
   {"id":"a92e...","user_id":"c49ea6a8-...","display_name":"Yusuf Demir","department":"Support",
    "rule":"overwork","state":"open","fired_at":"2026-09-19T06:15:00Z",
    "observed_seconds":45360,"against_seconds":28800,"subject_date":"2026-09-18",
    "resolved_at":null,"acknowledged_at":null,"acknowledged_by":null}]}
```

`?state=` takes `open` (the default), `acknowledged`, `resolved` or `all`.
Anything else is refused rather than read as `open`: a client asking for a
state this server does not have is a client with a bug, and a plausible answer
would hide it.

## The three rules

- **`no_agent_data`** — nothing has arrived from any of this person's live
  agents for longer than `alert_silence_hours`. In hours and about *now*,
  which makes it a different thing from the `no_data` signal
  ([Where to look](/kasl-server/reference/where-to-look/)): that one measures a
  person's own weekly rhythm and cannot speak before the week is complete.
  Both stamps count — a request that used the token, and a pulse
  ([Who is working now](/kasl-server/reference/who-is-working-now/)) — and the
  freshest wins. A machine whose employee is on holiday pulses `idle` all week
  and uploads nothing at all, and it is not silent.
- **`overwork`** — a finished day ran past `alert_overwork_factor` times that
  person's **own** norm for that date. A share, not a number of hours, so it
  means the same thing for somebody on half time; an installation-wide "over
  ten hours" would never mention a part-timer working double their day. The
  norm comes from the calendar and the rate
  ([The calendar and the norm](/kasl-server/reference/the-calendar-and-the-norm/)),
  so a short day before a holiday is a lower bar.
- **`day_not_closed`** — a day is still open after `alert_open_day_hours`.
  Judged by the wall clock and deliberately not against a norm: an open day
  has no total yet, so there is nothing to compare, and what is wrong with it
  is simply elapsed time. Usually kasl left running overnight — and the day it
  will eventually produce is wrong in a way that quietly poisons a week.

The two figures that are elapsed time - silence, and how long a day has been
open - have no upper bound, and the screen says them in the largest unit that
fits: `9 h`, `2 d`, `13 mo`. A real stand carried a day open since the
previous August, which is `9460.7 h` and is a number nobody converts. The
overwork figures stay in hours whatever their size, because they sit beside a
norm in the same sentence and two quantities being compared have to share a
unit.

A day whose norm is zero raises no overwork: a weekend, a holiday, a day of
leave or illness. Any work at all on such a day exceeds its norm by an
infinite share, and "you worked on your holiday" is between an employee and
their own screen.

Somebody with no live agent token raises nothing at all. An installation
halfway through handing tokens out would otherwise alert about every account
on its first afternoon, and the team table already says "no agents" in words.

## An alert is a record, not a calculation

The signals are not stored — they are a function of the workdays already in
the database, and a table of them would be a second copy of a derived fact.
Alerts **are** stored, and the difference is not an inconsistency: an alert
carries two things no workday can produce.

**When the condition began.** The absence of rows has no timestamp. A
recomputation can say a thing is true; only a row says since when.

**What a person decided about it.** `POST /api/v1/alerts/{id}/acknowledge`
records that somebody looked and it needs no action. The row stays, with who
answered it and when — "this was raised and a human decided it was fine" is
the only evidence that the thresholds are set somewhere sensible.

The server sweeps every five minutes and **reconciles**: it builds what ought
to be open now and moves the difference. A fortnight of silence is one row,
not four thousand, and an agent that comes back closes its own alert without
anybody clicking anything. A row that has fired never has its figures
rewritten — it says "13 h of silence" for as long as it lives, because that is
the statement that was true when somebody was told.

`acknowledged` and `resolved` are different facts and stay apart: a person
looked, versus the condition stopped being true on its own. An acknowledgement
suppresses its rule for that person **for exactly as long as the condition
lasts** — never forever, or one click would permanently mute a rule for one
employee with nothing on any screen saying so. The condition returning later
is a new event and a new row.

## The thresholds

`PUT /api/v1/alerts/thresholds` — administrators only, and audited.

```console
$ curl -X PUT -H "Cookie: kasl_session=..." -H "Content-Type: application/json" \
    -d '{"alert_silence_hours":8,"alert_overwork_factor":"1.4","alert_open_day_hours":14}' \
    http://127.0.0.1:8080/api/v1/alerts/thresholds
```

| Field | Default | Range |
| --- | --- | --- |
| `alert_silence_hours` | 12 | 1–720 |
| `alert_overwork_factor` | 1.50 | above 1, at most 5 |
| `alert_open_day_hours` | 16 | 1–168 |

These are settings, while the signals' thresholds are fixed in code — and the
difference is who gets interrupted. A signal is read by whoever opened the
page. An alert interrupts somebody, and how much silence is worth interrupting
over differs between a team in one timezone and a team across four. What is
**not** configurable is the set of rules: which things this server will speak
about is a choice, not an operator's to invent.

Every step an alert takes - raised, acknowledged, resolved - can also be sent
to a Slack, Mattermost or Telegram chat, or to a system of your own, in the
same transaction that took it. See
[Sending alerts to a chat](/kasl-server/guides/sending-alerts-to-a-chat/).
Notifications back to the employee's own kasl are a later milestone.
