---
title: What you are told
description: Notifications to the employee - the kinds, the agent's pull on the pulse, the two cursors, and the web inbox.
---

Everything else the server says, it says to a manager: [alerts on the
dashboard](/kasl-server/reference/what-needs-attention/), [webhooks into a
chat](/kasl-server/reference/webhooks/). Notifications are the other
direction - the server telling the person a fact is about, on their own
machine and in their own inbox.

## What is told

| `kind` | When | The facts beside the sentence |
| --- | --- | --- |
| `alert.raised` | The server raised an alert about you; your manager was told the same | `alert`: `rule`, `observed_seconds`, `against_seconds`, `subject_date`, `fired_at` |
| `agent.issued` | A new token can report as you - from the admin screen or `KASL_AGENTS` | `agent`: `id`, `name` |
| `agent.revoked` | A machine can no longer report as you | `agent`: `id`, `name` |
| `privacy.changed` | What this server keeps about your days changed | `privacy`: `from`, `to` |

Each notice carries a `title` and a `body` written by the server, in English,
next to its facts. An agent that does not know a `kind` shows the words and is
right; one that does can act on the facts - resend the day in
`alert.subject_date`, for an open day. The `body` may carry a command in
backticks:

> Your day of 2026-09-22 is still open here
>
> This server has had it open for 17 h, and your manager was told. If kasl
> closed the day on your machine, the close did not arrive: `kasl server push
> --date 2026-09-22` sends it again.

An acknowledgement of an alert is not told - "your manager decided it was
fine" is a comfort, not something to act on. Figures are the ones the alert
fired on and never change afterwards.

## On the machine

The server cannot reach an agent - laptops sleep behind NAT - so the agent
asks, on the minute it already reports in. The [pulse](/kasl-server/reference/who-is-working-now/)
answer carries how many notices this machine has not shown:

```console
$ curl -X POST -H "Authorization: Bearer $KASL_TOKEN" -H "Content-Type: application/json" \
    -d '{"state":"working","at":"2026-09-25T09:14:00-03:00"}' \
    http://127.0.0.1:8080/api/v1/agent/heartbeat
{"interval_seconds":60,"stale_after_seconds":180,"state":"working","clock_skew_seconds":0,"notifications":1}
```

Non-zero, and the agent reads them - oldest first, at most 20, `more` when
there is another page:

```console
$ curl -H "Authorization: Bearer $KASL_TOKEN" http://127.0.0.1:8080/api/v1/agent/notifications
{"notifications":[{"id":42,"kind":"alert.raised","created_at":"2026-09-25T12:10:04Z",
  "title":"Your day of 2026-09-22 is still open here","body":"This server has had it open for 17 h, ...",
  "withdrawn_at":null,"read":false,"link":"https://kasl.example.com/day",
  "alert":{"id":"7f0c...","rule":"day_not_closed","observed_seconds":61200,"against_seconds":57600,
           "subject_date":"2026-09-22","fired_at":"2026-09-25T12:10:04Z"}}],
 "more":false}
```

`link` is there when the server knows its own address (`KASL_PUBLIC_URL`).

Then it says how far it got. Two answers, because they are two facts:

- **`POST /api/v1/agent/notifications/ack`** `{"through": 42}` - this machine
  has shown everything up to 42. The cursor is this machine's: a second
  machine of the same person is still told, because a notice missed on the
  desk nobody is at is worse than the same notice twice.
- **`POST /api/v1/agent/notifications/read`** `{"through": 42}` - the person
  has seen it: they clicked the toast, or read the list in kasl. That is the
  same cursor the web inbox moves, and nothing they have read is toasted on
  any machine afterwards.

Both answer `{"through": n}` with where the cursor now stands. A cursor only
moves forward - a late acknowledgement cannot bring toasts back - and never
past the newest notice that exists for that person, so a number from the
future cannot silence notices not yet written. A negative `through` is `400`.

**What a machine is not told:**

- anything said before its token was issued - a new laptop does not replay a
  year;
- the notice that it was itself issued - "laptop can now report as you" is
  news to the desktop;
- a notice that is over: once the alert it announced has resolved - the close
  finally arrived - it is not toasted;
- its own silence. `no_agent_data` means nothing arrived from any of the
  person's machines, so a machine able to ask has, by asking, ended it. The
  notice is in the web inbox, where somebody away from every machine can read
  it.

## In the web UI

A bell in the header, on every screen, with the number of notices that are
unread and still true. Its screen lists the last hundred, newest first; a
notice that is over is kept and shown as over. Opening it is reading it: once
the list is on screen the cursor moves to the newest notice shown.

```console
$ curl -H "Cookie: kasl_session=..." http://127.0.0.1:8080/api/v1/me/notifications
{"notifications":[...],"unread":1,"read_through":41}
$ curl -X POST -H "Cookie: kasl_session=..." -H "Content-Type: application/json" \
    -d '{"through":42}' http://127.0.0.1:8080/api/v1/me/notifications/read
{"through":42}
```

**Yours alone.** No manager and no administrator can read another person's
notices; the facts behind them are already on their own screens - the alerts
feed, the audit log. The notices are listed in [the privacy
manifest](/kasl-server/concepts/what-the-server-stores-about-you/) at every
level, and they are in the backup.

The reasoning is in [ADR 0020](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0020-notifications-to-the-employee.md).
