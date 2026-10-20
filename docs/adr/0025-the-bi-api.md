# 0025. The BI API, and the tokens it is read with

Date: 2026-10-20

Status: Accepted

## Context

Companies that run a server like this one already have somewhere they look at
numbers - a Grafana next to their monitoring, a Power BI their finance team
lives in, a script that fills a sheet on Monday. Until now the hours could reach
them only as a file somebody downloaded by hand (ADR 0023), which is the opposite
of a dashboard.

Every piece of data such a tool wants was already served, to the web UI, under
`/api/v1`. Three things stood between that API and a dashboard.

**Who is asking.** Every route that answers for other people takes a session
cookie (ADR 0007). A dashboard tool cannot sign in, and handing it somebody's
password - or a session copied out of a browser, which dies after a fortnight
idle - is not a credential anyone should be asked to manage.

**What shape the answer is.** The web UI's answers are documents built for one
screen each: the timesheet nests every date inside every person beside the
month's columns (ADR 0024). A BI tool reads tables. Grafana's Infinity data
source and Power Query can flatten a document, one transformation per panel,
written by whoever builds the dashboard and broken by whoever renames a field.

**How long the shape holds.** `/api/v1` is versioned for kasl agents (ADR 0001):
the ingest routes keep meaning what they meant. The routes the web UI reads are
not held to that - they gain fields every release, and change shape when a
screen does, because the only client ships in the same binary. A dashboard
built on one of them is a second client nobody tests.

Metabase and other tools that read only databases were the fourth question, and
stay one; see the consequences.

## Decision

**A contract of its own: `/api/bi/v1`.** Versioned apart from the web UI's API,
with a rule the web UI's routes do not keep: within `v1` a dataset only ever
gains columns, at its end. A column is never renamed, removed, reordered or
given a new meaning; that is `/api/bi/v2`, with `v1` still answering. The
columns are listed once in the source, the reference page lists them, and the
test suite holds both to what was published - the published lists are copied
into the tests, so a rename in the code fails a test rather than quietly
rewriting the promise.

**Four flat datasets.** `people`, `departments`, `days` and `summary`: one row
per person, per department, per person and date, and per person over a range.
The same thing has the same name everywhere - `user_id`, `department_id` - so
the tables join without mapping, which is what a "join by field" in Grafana
needs. Each is served as JSON, an array of objects, and as CSV, by adding
`.csv` to the path: the same columns and the same values. Durations are whole
seconds, moments are UTC to the second, and nothing to say is `null` or an
empty field, never a zero (ADR 0015).

Seconds rather than hours because a BI tool formats a duration better than a
rounded figure can be unrounded; the exports round to hundredths of an hour
because they are read by people (ADR 0023), and these are read by programs.

**The server's rules, not a second copy.** `days` is the timesheet's cells over
any range - the month's builder was split into a range and a month, and both
ask the same function what a date was. `summary` is the team table's rows,
from the function behind the screen and the summary export. Who is in either
is decided by the one visibility clause (ADR 0009). Nothing about a day is
computed here that is not computed for the screens.

**Read tokens.** A table of their own, not a kind of agent. An agent's token
writes one person's days; a read token reads everybody its owner may see. In
one table the two would be a missing `WHERE` away from a dashboard that files
days under somebody's name - a class of defect, not a case.

- **It reads with its owner's eyes, as they are now.** A token is issued by a
  manager or an administrator, for themselves. Its owner's role and department
  are read on every request, through the same reader type a session produces,
  so a token narrows when its owner's authority narrows, answers `403` once
  they are an employee again, and `401` once they are deactivated. There is no
  second list of who a token may see to keep in step with the first.
- **Its scope is part of its type.** `read_scope` has one value, `hours`. The
  finance milestone adds `payroll`, so that the money can go to a dashboard
  without the hours going with it. A BI route takes `ReadToken<Hours>`, and a
  token of another scope is refused before the handler runs: a route cannot
  forget to check, because a route that names no scope has no reader.
- **It does not expire.** An expiry would be a setting nobody can choose well -
  short enough to matter is short enough to break a dashboard on a holiday -
  and a token already ends with its owner's access. It is revoked when it is no
  longer used, which `last_used_at` makes visible.
- **Its reads are not audited one by one.** A dashboard asks every few minutes;
  a line per request would bury every other entry in the log (ADR 0010). The
  entries are its issue and its revocation. The issue is what an export entry
  is for a file (ADR 0023): from that moment, the hours its owner may see can
  be read from wherever the token is.
- It is shown once, prefixed `kasl_read_`, and stored as a SHA-256 like an
  agent's (ADR 0004, ADR 0008).

## Consequences

- Grafana reads it through the Infinity data source with the token as a bearer
  token; Power Query and Power BI through a web source with the header;
  anything else that can send a header can too.
- **Metabase, Superset and other database-only tools cannot.** Metabase has no
  driver for an HTTP API. Serving them means a SQL door - a schema of views and
  a role that may read only them - and doing that honestly needs the norm and
  the state of a date to be defined where both SQL and Rust read them: today
  they are Rust (the calendar's rule and the timesheet's cell), and views that
  restated them in SQL would be a second copy free to disagree with the screens.
  It is left for a milestone of its own rather than built on a copy.
- A company dashboard dies with the account that issued its token. That is the
  rule doing its job; a dashboard meant to outlive its author is issued by an
  account meant to.
- `team::members`, the team's visibility check and the timesheet now take a
  reader rather than a signed-in user. A session and a token are two ways of
  arriving at the same reader.
- A backup carries the tokens, as it carries the agents': a restored server
  whose dashboards stopped answering would look broken for no reason anyone
  could see.
- The web UI has a screen for the tokens, reached from the export menu of the
  team and month screens: a token is that export kept going.
