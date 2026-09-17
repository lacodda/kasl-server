---
title: Who is working now
description: The agent heartbeat and the live team status it feeds - working, paused, idle, offline, unknown.
---

An agent reports in every minute with what it sees:

```console
$ curl -X POST -H "Authorization: Bearer $KASL_TOKEN" -H "Content-Type: application/json" \
    -d '{"state":"working","at":"2026-08-30T14:22:10-03:00"}' \
    http://127.0.0.1:8080/api/v1/agent/heartbeat
{"interval_seconds":60,"stale_after_seconds":180,"state":"working","clock_skew_seconds":0}
```

`state` is `working` (in a day, at the keyboard), `paused` (in a day, on a
break) or `idle` (the agent is running, the person is not in a day). `at`
carries the agent's own UTC offset. The server answers with the cadence it
wants rather than letting each agent pick one: report every
`interval_seconds`, and after `stale_after_seconds` of silence the pulse is no
longer believed.

A stamp more than a minute ahead of the server is refused with `400` rather
than accepted and corrected: a machine whose clock is wrong uploads hours that
are wrong too, and only its owner can fix that. `clock_skew_seconds` reports
the difference on every pulse so kasl can say so before it becomes a mystery.

The dashboard reads it back:

```console
$ curl -H "Cookie: kasl_session=..." http://127.0.0.1:8080/api/v1/team/live
{"poll_seconds":30,"stale_after_seconds":180,
 "members":[
   {"user_id":"c49ea6a8-...","status":"working","since_received":12},
   {"user_id":"0073460d-...","status":"offline","since_received":4210},
   {"user_id":"9f1c2b40-...","status":"unknown","since_received":null}]}
```

**`offline` and `unknown` are different answers.** `offline` means a machine
was reporting and stopped; `unknown` means no pulse has ever arrived - no
agent, or a kasl too old to send one. Neither is reported as `idle`, because
the server does not know that, and a dashboard that guessed would tell a
manager their whole team stopped working the day they rolled out an older
agent. Where there is no pulse, the row falls back to what `/team/days`
knows: "day open", "last data 20 min ago", "never reported".

**Its own endpoint, deliberately.** The week's hours are a page load; this is
polled every `poll_seconds` while the tab is open, and folding the two together
would re-run the heaviest query on the server on a timer. It applies the same
visibility rule as the rest of the team endpoints - who is at their keyboard
this minute is more sensitive than a week's totals, not less - and the web UI
stops polling entirely while its tab is hidden.

**Only the state is sent.** Not the task, not the reason for the break: those
belong to the day, under the privacy level that governs it. The pulse is
listed in [the privacy manifest](/kasl-server/concepts/what-the-server-stores-about-you/) at every
level, as the latest claim only - replaced each time, never kept as a history.
The reasoning is in [ADR 0014](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0014-the-agent-pulse.md).
