---
title: The BI API
description: GET /api/bi/v1 - the team's hours for Grafana, Power Query or a script. A read token, four flat datasets, JSON or CSV, and columns that stay where they are.
---

A dashboard tool cannot sign in, and the web UI's own API changes shape with
its screens. `/api/bi/v1` is the door for both problems: a **read token**
instead of a session, and four flat tables whose columns stay put for as long
as the path says `v1`.

| Route | What it answers |
| --- | --- |
| `GET /api/bi/v1/people` | one row per person the token may see |
| `GET /api/bi/v1/departments` | one row per department |
| `GET /api/bi/v1/days?from=YYYY-MM-DD&to=YYYY-MM-DD` | one row per person and date of the range, recorded or not |
| `GET /api/bi/v1/summary?from=YYYY-MM-DD&to=YYYY-MM-DD` | one row per person: the range's totals |

Add `.csv` to any of them - `/api/bi/v1/days.csv?...` - for the same table as
CSV. Every request carries the token:

```console
$ curl -H "Authorization: Bearer kasl_read_9f0c..." \
    "https://kasl.example.com/api/bi/v1/summary?from=2026-08-01&to=2026-08-31"
[{"user_id":"c49ea6a8-...","display_name":"Anna","email":"anna@example.com",
  "department_id":"5b1e...","department":"Engineering","active":true,"work_rate":1.0,
  "days_worked":21,"days_away":1,"worked_seconds":612000,"paused_seconds":63000,
  "norm_seconds":604800,"due_seconds":604800,"last_day":"2026-08-31"}, ...]
```

Building a dashboard step by step is in
[reading hours into Grafana](/kasl-server/guides/reading-hours-into-grafana/).

## Read tokens

A read token belongs to the manager or administrator who issued it, and reads
**exactly what they may see, as of each request**: an administrator's reads
everybody, a manager's reads themselves and the departments they run (the rule
in [managing the team](/kasl-server/guides/managing-the-team/)). Move the
manager to another department and the token follows; make them an employee
again and it answers `403`; deactivate them and it answers `401`, like a token
that never existed.

Issue one in the web UI - **Export → Read from a dashboard tool** on the team
or month screen - or with the API, signed in:

| Route | Who |
| --- | --- |
| `GET /api/v1/read-tokens` | managers: their own tokens; administrators: everyone's |
| `POST /api/v1/read-tokens` with `{"name": "Grafana", "scope": "hours"}` | managers and administrators, for themselves |
| `DELETE /api/v1/read-tokens/{id}` | the token's owner, or an administrator |

A token's **scope** says what it reads, and is required. `hours` reads this
API. `metrics` reads the server's own health at `/metrics` - an
administrator's alone; see [server metrics](/kasl-server/reference/server-metrics/).
Each is refused at the other's door with `403`.

The token is answered once, at issue, and starts with `kasl_read_`; the server
keeps only its SHA-256. It does not expire - revoke it when it is no longer
used, which the list's `last_used_at` makes easy to see. Revoking twice is
still revoked, and does not move the moment access ended.

A read token is not an agent's token. It cannot upload a day, and an agent's
token cannot read the team: each is refused at the other's door with `401`.

**What is recorded.** Issuing and revoking a token are in
[the audit log](/kasl-server/reference/the-audit-log/) as `read_token.issued` and
`read_token.revoked`, with its name and scope. The reads are not: a dashboard
asks every few minutes, and a line per request would bury everything else in the
log. The issue entry stands for them - from that moment, the hours its owner may
see can be read from wherever the token is.

## Formats

- **JSON** is an array of flat objects, keys in the order of the tables below.
- **CSV** is the same table: a header row of the column names, one line per
  row, UTF-8 with CRLF line ends and no byte-order mark. Text that begins with
  `=`, `+`, `-` or `@` gets a leading apostrophe, as in
  [the exports](/kasl-server/reference/exporting-hours/), so a spreadsheet does
  not run it as a formula.
- The same value is written the same way in both. Durations are **whole
  seconds** - a BI tool formats them as hours better than a rounded figure can
  be unrounded. Dates are `YYYY-MM-DD`; moments are RFC 3339 in UTC to the
  second (`2026-08-24T12:00:00Z`). Nothing to say is `null` in JSON and an empty
  field in CSV, and never a zero.
- Answers carry `Cache-Control: no-store`: they are people's hours.

A range is at most 400 days, `to` not before `from`; otherwise `400` with the
reason. There is no paging: a range and a team bound the answer.

## What stays the same

**Within `v1`, a dataset only gains columns, at its end.** A column is never
renamed, removed, reordered or given a different meaning - that would be
`/api/bi/v2`, with `v1` still answering. Read columns by name and a new one at
the end changes nothing for you. The names and values of a state, kind or
status are the API's own and are part of the contract too.

## The datasets

The same thing has the same name everywhere, so the tables join without
mapping: `days.user_id` is `people.user_id`, and `department_id` is a
department's in every table that has one. Join on ids rather than names - a
department can be renamed.

### `people`

Everyone the token may see, deactivated people included, so a day's `user_id`
always finds its person.

| Column | Type | What it is |
| --- | --- | --- |
| `user_id` | uuid | The person. |
| `display_name` | text | As the web UI shows them. |
| `email` | text | |
| `role` | text | `admin`, `manager` or `employee`. |
| `active` | boolean | `false` once the account is deactivated. |
| `department_id` | uuid, nullable | Their department; `null` when unfiled. |
| `department` | text, nullable | Its name. |
| `work_rate` | number | Their share of a full day: `1`, `0.5`. See [the norm](/kasl-server/reference/the-calendar-and-the-norm/). |
| `created_at` | moment | When the account was made. |

### `departments`

Every department. That a company has a "Sales" is not the disclosure - the
people in it are, and `people` scopes them.

| Column | Type | What it is |
| --- | --- | --- |
| `department_id` | uuid | The department. |
| `department` | text | Its name. |
| `manager_id` | uuid, nullable | Who runs it; `null` between heads. |
| `manager` | text, nullable | Their display name. |

### `days`

Every date of the range for every person the token may see - the
[timesheet's](/kasl-server/reference/the-timesheet/) cells, over any range, with
the day under each. Who is in it is who the summary lists: everyone active the
token may see, and anyone deactivated since who has days in the range. A date
owed and never recorded is a row saying so, which is what a chart of missing
days is drawn from.

| Column | Type | What it is |
| --- | --- | --- |
| `date` | date | The date, in the person's own calendar, as their agent recorded it. |
| `user_id` | uuid | The person. |
| `display_name` | text | Repeated on every row, so a chart needs no join. |
| `email` | text | |
| `department_id` | uuid, nullable | |
| `department` | text, nullable | |
| `state` | text | `worked`, `open` (still running), `away`, `unrecorded` (owed, come due, nothing recorded), `ahead` (owed, not come due) or `free` (owed nothing). |
| `kind` | text, nullable | `work`, `vacation`, `sick` or `day_off`; `null` where nothing was recorded. |
| `started_at` | moment, nullable | When the day began. |
| `ended_at` | moment, nullable | When it ended; `null` while it runs. |
| `worked_seconds` | integer, nullable | The span less what was paused; `null` for an open day and where nothing was recorded. |
| `paused_seconds` | integer, nullable | What was paused; known for an open day too. |
| `norm_seconds` | integer | What the date asked of this person: the calendar at their share of a full day, `0` on a day away. |
| `filed` | text, nullable | Whether the day was reported in time: `on_time`, `late`, `missing` or `pending`. See [reports](/kasl-server/reference/reports-and-approval/). |
| `report` | text, nullable | The newest report's status: `submitted`, `approved`, `returned` or `changed`. |
| `calendar` | text, nullable | What the calendar says about the date: `holiday`, `short_day` or `working_weekend`. |
| `weekend` | boolean | Saturday or Sunday, before the calendar has its say. |

### `summary`

The range's totals per person - the row the team table and
[the summary export](/kasl-server/reference/exporting-hours/) show for the same
range. A person's `days` rows add up to it.

| Column | Type | What it is |
| --- | --- | --- |
| `user_id` | uuid | The person. |
| `display_name` | text | |
| `email` | text | |
| `department_id` | uuid, nullable | |
| `department` | text, nullable | |
| `active` | boolean | |
| `work_rate` | number | Their share of a full day. |
| `days_worked` | integer | Days of work recorded. |
| `days_away` | integer | Days of leave, illness or time off. |
| `worked_seconds` | integer | Across finished days; an open day adds nothing yet. |
| `paused_seconds` | integer | |
| `norm_seconds` | integer | What the whole range asks. |
| `due_seconds` | integer | The part of the norm that has come due - what a range still running is measured against. |
| `last_day` | date, nullable | The latest date with anything recorded. |

## Tools that read only databases

Metabase, Superset and other tools that query a database rather than an HTTP
API cannot read this directly. Grafana can, through its Infinity data source;
so can Power Query in Excel and Power BI (**Get data → Web**, with the
`Authorization` header), and anything that can send a request with a header.

The reasoning behind all of this is in
[ADR 0025](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0025-the-bi-api.md).
