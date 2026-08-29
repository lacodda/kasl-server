# 0013. The demo mode: a fictional team on an empty database

Date: 2026-08-29

Status: Accepted

## Context

Fourteen versions in, the server has an ingest contract, roles, departments,
an audit log, a privacy manifest, a manager's dashboard and an employee's own
week - and no way to see any of it without first installing kasl on somebody's
machine, issuing a token, and waiting for days to accumulate. The install
guide gets a stranger to an empty dashboard in a minute; the empty dashboard
is where the trial ends.

A self-hosted product is sold by being tried. What is missing is a server
that has something on it.

## Decision

**One environment variable.** `KASL_DEMO=true` makes the server fill an empty
database with a fictional team on startup and print the logins. It works for
`docker compose up` and for `cargo run` alike, because both already read the
environment; a subcommand would have needed a different entrypoint in the
container, and the install path would have stopped being one command.

**The team is generated, not stored.** Three departments, twelve people, an
agent for everyone but the administrator, and eight weeks of weekdays ending
yesterday. The generator is deterministic for a given date, using its own
small xorshift rather than `rand`, whose output for a seed is not promised to
survive a version bump: two demos started on the same day show the same
numbers, and a screenshot can be reproduced. The history is relative to
"today" so the dashboard's default week is never empty, however old the
release.

The days are shaped so that every state the dashboard can render is on screen
at once: someone steady, someone working ten-hour days, someone whose hours
shrink week by week, a day open right now, an agent that went silent a week
ago, one that never reported, and an administrator with no agent at all. A
demo whose rows all look the same shows the product's charts and none of its
judgement.

**History goes through the import path.** Every day is written by the same
function an operator's `kasl-server import` uses, so the demo exercises the
rows the dashboards were built on rather than a parallel INSERT that could
drift from them. The one difference is the transaction: an import commits each
day so it can fail halfway and resume; the demo commits a person at a time,
because four thousand commits was the difference between a demo that opens in
seconds and one that takes minutes.

**A populated database is refused.** With `KASL_DEMO` set, a database that
already holds accounts - and is not the demo - stops the server with a message
naming what is in the way. The alternative, seeding the fictional people
alongside the real ones, would put twelve invented employees on a real team's
dashboard the day someone forgot to remove the flag after a trial, and nothing
would say which rows were which. A database the demo itself seeded starts
normally.

**The mark lives in the database.** `settings.demo` is set when the team is
seeded and read by `/health`, which the web UI asks before anyone signs in.
The label therefore outlives the environment variable: drop `KASL_DEMO`, and
the banner saying "nothing here is real" stays, because the data is still
invented. A backup carries the mark, and a restore keeps it; a backup from
before the column existed restores as a real installation, which it was.

**Fixed passwords, listed on the login screen.** Every account signs in with
`kasl-demo`, and `GET /api/v1/demo/accounts` - answered only on a demo, a 404
everywhere else - lists one account per role so the login page can offer "try
it as a manager" as a single click. A demo server has nothing to protect; a
visitor who has to dig three generated passwords out of a log before seeing a
dashboard has been handed a chore. The agents' tokens are fixed for the same
reason, so a real kasl can be pointed at the demo and its days appear next to
the invented ones.

**Every address is under `example.com`.** The names are invented; the domain,
reserved by RFC 2606, guarantees the addresses are too.

## Consequences

- `KASL_DEMO` is documented in the configuration table and passed through
  `docker-compose.install.yml`, off by default.
- A new migration adds `settings.demo`. `/health` gains a `demo` field. The
  restore path copies the flag, defaulting to `false` for older files.
- The web UI shows a banner on every screen of a demo, including the login,
  and offers the showcased accounts there.
- The seeding is recorded in the audit log as `demo.seeded`, with no actor:
  it is a thing the server did.
- Changing the team - a name, a pattern, the number of weeks - changes what
  every future demo shows. The unit tests pin the properties that matter (one
  of every role, one of every pattern, weekdays only, determinism), not the
  exact numbers.
