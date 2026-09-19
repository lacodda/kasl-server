---
title: Where to look
description: GET /api/v1/team/signals - a three-week slide, a quiet agent, or a week far from someone's own median.
---

`GET /api/v1/team/signals` answers the question a manager does not know to
ask. A three-week slide exists only across weeks; no single screen shows it,
and nobody scrolls back through weeks hunting for one.

```console
$ curl -H "Cookie: kasl_session=..." http://127.0.0.1:8080/api/v1/team/signals
{"from":"2026-06-08","to":"2026-08-30","people":12,
 "signals":[
   {"user_id":"0073460d-...","display_name":"Jonas Petit","department":"Design",
    "kind":"no_data","days_quiet":13,
    "weeks":null,"from_seconds":null,"to_seconds":null,"median_seconds":null},
   {"user_id":"c49ea6a8-...","display_name":"Lukas Brandt","department":"Engineering",
    "kind":"declining","weeks":3,"from_seconds":97200,"to_seconds":79560,
    "median_seconds":null,"days_quiet":null}]}
```

**Everything here compares a person with themselves.** Never with a colleague
and never with a norm - this server has none until the production calendar
(v0.21), and a threshold invented before then would be this product asserting
what a working day should be on somebody else's team. A dashboard that ranked
people against each other would be a scoreboard, which this deliberately is
not.

**A signal is a question, not a verdict.** Each one carries the figures it was
computed from, so a screen says "27 h a week down to 22 h" rather than showing
a badge that reads "problem". Falling hours are a holiday, a hospital, or a
project that ended, and the server knows none of that.

The three:

- **`declining`** — the last three weeks sit at least 15 % below the three
  before them, comparing the median of each side. **Levels, not a run of
  falls:** a genuinely fading person goes 33 → 24.8 → 27 → 20.9 → 23.1 → 22.1,
  which is an unmistakable slide and never three consecutive drops. One
  ordinary week resets a run, so counting steps stays silent on exactly the
  case this is for ([ADR 0016](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0016-signals-and-the-weekly-trend.md)).
- **`no_data`** — nothing recorded for ten days or more. Measured in whole
  days against a person's own rhythm, so it cannot speak before the week is
  out; the `no_agent_data` alert is the one that answers in hours. The live status
  ([Who is working now](/kasl-server/reference/who-is-working-now/)) only sees a silent machine once a pulse is
  expected; an agent too old to send one is invisible there.
- **`unusual_week`** — the last complete week is more than 40 % away from that
  person's own median, in **either** direction. Both ways on purpose: flagging
  only the short weeks would make the signal an accusation rather than a
  question.

**The current week never enters the arithmetic.** A Tuesday is not a short
week, but that is what a partial week looks like to a sum - and including it
would flag the whole team every Monday morning. Medians rather than means
throughout, so one crunch week cannot hide a decline behind it.

`GET /api/v1/users/{id}/trend` is what a signal links to: twelve complete
weeks, the empty ones included, plus that person's median and the signals
about them. A week nobody worked keeps its place — closing the gap up would
turn an absence into continuity.

Nothing here is stored and nothing is sent: the signals are a function of the
workdays already in the database. What the server notices *without* being
asked — and remembers, and lets somebody answer — is a different object, and
it lives at [What needs attention](/kasl-server/reference/what-needs-attention/).
