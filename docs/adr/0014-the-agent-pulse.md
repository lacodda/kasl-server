# 0014. The agent pulse and the live dashboard

Date: 2026-08-30

Status: Accepted

## Context

Every endpoint before this one answers about the past. A day is uploaded when
it closes, or now and then while it is open, so the manager's table could say
"last data 2 h ago" and nothing more. That is honest, and it is not what the
screen is for: the question a team dashboard exists to answer is who is
working right now, and the server had no way to know.

`agents.last_seen_at` is the closest thing that existed - a stamp written
whenever a token was used. It cannot carry the weight. An agent backfilling
last month and an employee at their keyboard produce the same stamp, so
reading it as presence would report somebody as working because their machine
uploaded a year of history at midnight.

The agent knows the answer. kasl runs a watcher; it knows whether it is inside
a pause, and whether the day is open. Nothing else does, and nothing else can
without inferring it from rows that arrive minutes late.

## Decision

**The agent claims a state; the server times it.** `POST /api/v1/agent/heartbeat`
takes `{state, at}` where `state` is `working`, `paused` or `idle`, and `at`
carries the agent's own UTC offset like every other timestamp in this API
(ADR 0003). The claim is the agent's because only it knows; the ageing is the
server's, measured against `heartbeat_received_at`, its own clock. Trusting the
agent's stamp for staleness would let a machine an hour fast look alive for an
hour after it stopped, and one an hour slow look permanently offline.

A stamp more than a minute in the future is refused rather than clamped.
Clamping stores a plausible row and hides the real fault: a clock that is
wrong makes every hour that machine uploads wrong too, and only its owner can
fix it. The response carries the measured skew for the same reason.

**One row per agent, never a log.** The pulse lands on `agents`, replacing
what was there. A `heartbeats` table would be the largest thing in the
installation within a week, and it would be a minute-by-minute record of when
each employee was at their desk - a different product from a time tracker, and
one this is deliberately not. What the server keeps is the latest claim and
nothing behind it.

**Silence is not a state.** `LiveStatus` is wider than what an agent can send:
`offline` is a pulse too old to believe, `unknown` is no pulse at all. They
are kept apart because they mean different things - `offline` says a machine
stopped answering, `unknown` says the server never heard from one, which on a
team still rolling kasl out is most of the table and is not evidence about
anybody. Reading either as `idle` would be the server inventing an answer, the
same defect the privacy work fixed at ingest (ADR 0011).

**Three intervals of margin.** The agent is asked to report every 60 seconds
and a pulse is believed for 180. Two missed pulses are a laptop lid or a slow
link, and a dashboard that flickered "offline" at people who are working would
be worse than none. The agent is told both numbers on every pulse rather than
configuring its own, because the interval and the threshold have to agree and
only one side can own that.

**A separate endpoint for the dashboard.** `GET /api/v1/team/live` answers a
status per user id, and nothing else. The week's hours are a page load; the
pulse is a poll every thirty seconds, and folding it into `/team/days` would
re-run the heaviest query on the server on a timer for figures that did not
change. The feed applies `VISIBLE_USERS`, the same clause as the rest of
`team` - who is at their keyboard this minute is more sensitive than a week's
totals, not less.

**Polling, not a stream.** Server-sent events would hold a connection per
viewer and break on proxy timeouts, to save thirty seconds. The client polls
at half the agent's interval and stops entirely while the tab is hidden, so a
dashboard left open overnight asks nothing until somebody looks at it.

**The manifest names it.** The pulse is listed in the privacy manifest at
every level, worded as what it is: the latest claim only, replaced each time,
never kept as a history. The privacy level governs what is stored about days,
not whether the agent reports the present moment - so a manifest that
described only days would be describing a quieter server than the one running.

## Consequences

- A migration adds `agents.heartbeat_state` (enum `agent_state`),
  `heartbeat_at`, `heartbeat_received_at`, and `demo_pulse_age_seconds`.
- Older agents send no pulse and read as `unknown`. Nothing breaks for them,
  and the dashboard keeps the pre-pulse row - "day open", "last data 2 h ago",
  "never reported" - as the fallback when there is no pulse to show.
- The demo seeds pulses, including two agents held deliberately stale so
  "this machine stopped answering" is on screen, and re-stamps them on a timer
  for as long as it runs. Without the timer the whole live column would age
  out three minutes after the demo started. The age is recorded per row rather
  than inferred, because a pulse that merely aged is indistinguishable from one
  seeded old - and the same column keeps the refresh off any real agent
  pointed at the demo.
- kasl's paired milestone is v3.5. Until it ships, the endpoint has no client
  in the wild: the contract is exercised by this repository's tests rather
  than by a real agent, which is a weaker check than the ingest contract got
  and is recorded here as such.
