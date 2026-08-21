<p align="center"><img src="https://github.com/lacodda/kasl-server/raw/main/assets/banner.svg" alt="kasl-server - team server for kasl" width="720"></p>

# kasl-server

Team server for [kasl](https://github.com/lacodda/kasl). Employees run kasl on their machines; the agents send work-time data to the server. Managers get dashboards, charts, and reports across the whole team; every employee gets a personal page.

> **Status: pre-alpha.** The door for kasl agents is open and survives a bad connection: a day at a time on `POST /api/v1/days`, a backlog on `/days/batch`, and a task the employee deleted can be deleted here too. History from before the server arrived can be imported from an agent's own database; people sign in, and an administrator manages the team, its departments and its agent tokens without touching the host. The tables are filled; almost nothing reads them back yet — dashboards and the personal page are the next milestones, and there is still nothing to deploy for real use.

## Try it

Requires Rust and Docker.

```console
$ git clone https://github.com/lacodda/kasl-server && cd kasl-server
$ docker compose up -d db
$ export DATABASE_URL=postgres://kasl:kasl@localhost:5433/kasl
$ export KASL_AGENTS=employee@example.com:agent-token
$ cargo run
2026-08-21T21:17:10.598497Z  INFO kasl_server: database schema is up to date version=20260821000001
2026-08-21T21:17:10.664121Z  INFO kasl_server::provision: provisioned agents from KASL_AGENTS agents=1
2026-08-21T21:17:10.666807Z  INFO kasl_server: kasl-server listening version="0.8.0" addr=0.0.0.0:8080 max_batch_days=31 max_body_bytes=4194304

$ curl http://127.0.0.1:8080/health
{"database":"ok","status":"ok","version":"0.8.0"}

# An agent back from three days offline. The middle day is impossible - it ends
# before it starts - and the others land anyway.
$ curl -X POST http://127.0.0.1:8080/api/v1/days/batch \
    -H "Authorization: Bearer agent-token" -H "Content-Type: application/json" \
    -d '{"days":[
         {"date":"2026-08-15","started_at":"2026-08-15T09:04:00-03:00","ended_at":"2026-08-15T18:12:00-03:00",
          "tasks":[{"agent_task_id":7,"recorded_at":"2026-08-15T18:10:00-03:00","name":"Reliable ingest","completeness":60}]},
         {"date":"2026-08-16","started_at":"2026-08-16T19:00:00-03:00","ended_at":"2026-08-16T09:00:00-03:00"},
         {"date":"2026-08-17","started_at":"2026-08-17T09:11:00-03:00","ended_at":"2026-08-17T17:40:00-03:00",
          "tasks":[{"agent_task_id":7,"recorded_at":"2026-08-17T17:38:00-03:00","name":"Reliable ingest","completeness":100}],
          "tasks_are_complete":true}]}'
{"accepted":2,"rejected":1,"results":[
  {"status":"accepted","workday_id":"82ca500d-feeb-4d1f-8fb7-0b376339be02","date":"2026-08-15","pauses":0,"tasks":1,"deleted_tasks":0},
  {"status":"rejected","date":"2026-08-16","error":"ended_at is before started_at"},
  {"status":"accepted","workday_id":"2ea46d40-3aa0-48d3-8d8d-e1bb152a36bc","date":"2026-08-17","pauses":0,"tasks":1,"deleted_tasks":0}]}

# The employee deletes the task in kasl; the agent re-sends the day and says so.
$ curl -X POST http://127.0.0.1:8080/api/v1/days \
    -H "Authorization: Bearer agent-token" -H "Content-Type: application/json" \
    -d '{"date":"2026-08-17","started_at":"2026-08-17T09:11:00-03:00","ended_at":"2026-08-17T17:40:00-03:00",
         "tasks":[],"tasks_are_complete":true}'
{"workday_id":"2ea46d40-3aa0-48d3-8d8d-e1bb152a36bc","date":"2026-08-17","pauses":0,"tasks":0,"deleted_tasks":1}
```

The task is gone from the 17th - and still there on the 15th, where the employee
did not delete it.

The dev database listens on 5433, leaving a PostgreSQL you may already run on
5432 alone; override with `KASL_DB_PORT`.

## The API

`/api/v1` from the first endpoint: agents update on their own schedule, so a
path keeps meaning what it meant when the agent calling it shipped.

**`POST /api/v1/days`** — upload one day. Requires `Authorization: Bearer
<token>`. The body is the workday with its pauses and tasks; `ended_at` is
absent while the day is still running, and so is a pause's, and `tasks` may be
empty.

Two properties are worth knowing before writing a client:

- **Every instant needs a UTC offset**, and the day carries its own `date`.
  `2026-08-14T09:12:00` without an offset is refused (422): one team's hours
  have to stay comparable across time zones, and which calendar day work
  belongs to is the agent's call, not a value derived on the server.
- **The last upload wins.** Re-sending a day replaces what is stored, so a
  correction made in kasl lands and a retry after a lost connection is safe -
  the same payload twice leaves the same rows. Pauses are replaced as a set;
  tasks are matched on `agent_task_id`, so a task carried into the next day
  moves rather than multiplying.
- **Deleting a task takes one word.** Send `"tasks_are_complete": true` and the
  date's tasks the payload omits are deleted, which is how a task the employee
  removed in kasl disappears here too. Other dates are untouched. Leave the flag
  out - as agents written before it did - and nothing is ever deleted.

**`POST /api/v1/days/batch`** — upload a backlog. The body is `{"days": [...]}`
with the same day objects, and the answer reports each one:

```json
{"accepted": 2, "rejected": 1, "results": [
  {"status": "accepted", "date": "2026-08-10", "workday_id": "...", "pauses": 1, "tasks": 3, "deleted_tasks": 0},
  {"status": "rejected", "date": "2026-08-11", "error": "ended_at is before started_at"},
  {"status": "accepted", "date": "2026-08-12", "workday_id": "...", "pauses": 0, "tasks": 1, "deleted_tasks": 0}
]}
```

Each day is written on its own, so one the server will never accept does not
block the rest - an agent that could not deliver *any* of its backlog because of
a single bad row would retry the same request forever. The batch carries at most
`KASL_MAX_BATCH_DAYS` days (31) and the body at most `KASL_MAX_BODY_BYTES`
(4 MiB); past either, `413`.

**Which failures are worth retrying.** `4xx` means the payload will not be
accepted as sent, however many times it is tried - fix it or drop it. `5xx`
means the fault is on this side; send it again later. A batch that answers `5xx`
stopped partway: the days already accepted are stored, and re-sending them is
safe because the last upload wins.

A malformed day is refused with `400` and a reason naming the field
(`{"error":"tasks[0]: completeness must be between 0 and 100"}`); an
unrecognized, revoked or deactivated token gets `401`. The reasoning behind all
of this is in [ADR 0004](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0004-the-ingest-contract.md)
and [ADR 0005](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0005-deletions-and-backfill.md).

## Signing in

People sign in with an email and a password; kasl agents keep using their bearer
token and are unaffected by any of this.

```console
$ kasl-server admin --email boss@example.com --password '...'
admin boss@example.com is ready

$ curl -i -X POST http://127.0.0.1:8080/api/v1/auth/login     -H "Content-Type: application/json"     -d '{"email":"boss@example.com","password":"..."}'
HTTP/1.1 200 OK
set-cookie: kasl_session=b7857303342e1a4b...; Path=/; HttpOnly; SameSite=Strict; Max-Age=1209600
{"status":"ok"}

$ curl -H "Cookie: kasl_session=b7857303342e1a4b..." http://127.0.0.1:8080/api/v1/auth/me
{"id":"de836432-5dce-4705-9344-a65b356fc662","email":"boss@example.com","display_name":"boss@example.com","role":"admin"}
```

`POST /auth/logout` ends this session, `POST /auth/logout-everywhere` ends all of
them, and `GET /auth/me` says who the caller is.

Sessions are rows in the database, not signed tokens. The query per request buys
the thing a self-contained token cannot give: access ends when it is ended — the
afternoon someone leaves, not whenever their token happens to expire. A session
lasts a fortnight and each use pushes that out again.

An unknown email, a deactivated account and a wrong password all answer
`{"error":"wrong email or password"}`, so the login form cannot be used to find
out who works somewhere.

**The first administrator** comes from `kasl-server admin` or
`KASL_ADMIN=email:password` in the environment. Running it again resets the
password and promotes the account, which is both the way back in after a
forgotten password and the way to make an admin of someone whose account already
exists because their agent has been reporting. Accounts created by `KASL_AGENTS`
have no password and cannot be signed into — they exist to own an agent's data.

Set `KASL_SECURE_COOKIES=false` when serving over plain `http://`. A `Secure`
cookie is silently dropped by the browser there, which looks exactly like login
doing nothing. The reasoning is in
[ADR 0007](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0007-sessions-and-the-first-admin.md).

## Managing the team

Once an administrator exists, people and agent tokens are managed over the API
rather than through the host's environment.

```console
$ curl -X POST http://127.0.0.1:8080/api/v1/users -H "Cookie: kasl_session=..."     -H "Content-Type: application/json"     -d '{"email":"ivan@example.com","display_name":"Ivan","password":"..."}'
{"id":"9b5c1fd8-cf3d-433e-bb9e-0c2bf1c1cfac"}

$ curl -X POST http://127.0.0.1:8080/api/v1/users/9b5c1fd8-.../agents     -H "Cookie: kasl_session=..." -H "Content-Type: application/json"     -d '{"name":"ivan-laptop"}'
{"id":"b749b090-db08-464d-b48d-4fe15f7acc43","name":"ivan-laptop",
 "token":"kasl_<64 hex chars>",
 "notice":"this token is shown once; the server keeps only its hash"}

$ curl -X DELETE http://127.0.0.1:8080/api/v1/agents/b749b090-... -H "Cookie: kasl_session=..."
# 204; the same token now gets 401 from the ingest routes
```

| Route | Who |
| --- | --- |
| `GET /users`, `GET /users/{id}/agents` | admin (everyone), manager (their departments) |
| `GET /departments` | admin, manager |
| `POST /departments`, `PATCH`/`DELETE /departments/{id}` | admin |
| `PUT /users/{id}/department` | admin |
| `POST /users`, `PATCH /users/{id}` | admin |
| `POST /users/{id}/agents`, `DELETE /agents/{id}` | admin |
| `POST /auth/password` | anyone signed in, for their own password |

**A manager reads their departments and changes nothing.** A department names its
manager, and a person belongs to one:

```console
$ curl -X POST http://127.0.0.1:8080/api/v1/departments -H "Cookie: kasl_session=..."     -H "Content-Type: application/json"     -d '{"name":"Engineering","manager_id":"d7c9ef3a-..."}'
{"id":"997c3947-4028-45e6-9c1c-9cd334b10c5d"}

$ curl -X PUT http://127.0.0.1:8080/api/v1/users/<id>/department -H "Cookie: kasl_session=..."     -H "Content-Type: application/json" -d '{"department_id":"997c3947-..."}'
# 204; `{"department_id":null}` takes them out again without deleting anything
```

The manager of Engineering sees the people in Engineering, plus themselves — a
manager who runs nothing yet would otherwise get an empty page and think the
product was broken. An administrator sees everyone.

**Someone with no department is visible to the administrator alone.** Showing
the unfiled to every manager, so nobody gets lost, fails in the direction nobody
observes: forget to file a person and they are exposed company-wide, silently.
Missing from a list is reported the same afternoon.

Deleting a department leaves its people unfiled rather than deleting them, and
an employee cannot be made to run one — they could not see it, so it would
silently have no working head. Issuing an agent token stays with the
administrator: it is the authority to write someone's history, and there is no
audit log until the next milestone.

**An administrator sets an initial password and hands it over; the person
changes it** with `POST /auth/password`, which requires the current one. The
server has no mail channel, so there is nothing to send an invite link to that
would not be handed over the same way a password is.

Some things follow from a change rather than being asked for separately:

- Deactivating someone, or resetting their password, deletes their sessions.
- Changing your own password ends every *other* session and keeps the one you
  are using.
- The last administrator cannot be demoted or deactivated — the only way back
  from that is the `admin` subcommand on the host.
- A user is never deleted, only deactivated: their days have to keep an owner.

Agent tokens are shown once and stored as a SHA-256. The reasoning is in
[ADR 0008](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0008-roles-and-agent-tokens.md)
and [ADR 0009](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0009-departments-and-visibility.md).

## Importing history from before the server

Someone can track their time with kasl for a year before their team runs a
server. That history is an ordinary SQLite file on their machine, and it does
not have to be lost because the server arrived second:

```console
$ kasl-server import --db kasl.db --user employee@example.com --timezone -03:00
read 240 workdays, 312 pauses, 460 tasks from kasl.db
skipped 17 tasks the employee had deleted
imported 240 days as employee@example.com at -03:00
```

`--timezone` is required and has no default. kasl stores bare wall-clock text,
so nothing in the file says which offset it was recorded in - and a wrong guess
produces a perfectly plausible-looking year of work at the wrong hour. The
answer comes from whoever knows, and is echoed back so it is on the record.

- `--dry-run` reads and reports without writing anything.
- `--since` / `--until` bound the import by date, both ends inclusive. This is
  how someone who moved between time zones is imported correctly: one run per
  stretch, each with the offset that stretch was recorded in.
- The account must already exist - an import will not create it, so a typo in
  the email address cannot quietly file a year of history under a stranger.
- Re-importing replaces rather than duplicates, so a run that failed partway can
  simply be repeated, and a wrong offset is fixed by importing again with the
  right one.

The agent's file is opened read-only and never written to. Details and the
trade-offs behind the fixed offset are in
[ADR 0006](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0006-importing-local-history.md).

## The data model

Migrations live in `migrations/` and are applied on startup. The shape follows
kasl's own model, so a reader who knows the agent recognizes it:

| Table | Holds |
| --- | --- |
| `users` | People and their role: admin, manager, employee |
| `sessions` | Browser sign-ins; a token hash each, never the token |
| `departments` | Groups of people, each naming the manager who runs it |
| `agents` | Installed kasl instances; a token hash each, never the token |
| `workdays` | One row per person per date: when the day started and ended |
| `pauses` | Idle stretches and manual breaks inside a day |
| `tasks`, `tags`, `task_tags` | What was worked on, and how it is labelled |
| `reports` | That a report was submitted, when, and with which figures |

Two differences from the agent's database are deliberate: instants are stored
with a time zone (the agent stores bare wall-clock text, which does not survive
a team spread across zones), and rows are tied together by foreign keys rather
than by comparing dates. Both are recorded in [ADR 0003](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0003-time-and-identity-in-the-schema.md).

## Running it somewhere real

`docker-compose.prod.yml` builds the server and starts it next to PostgreSQL.
The image is built on the machine that will run it, so a stand on a Raspberry
Pi gets an aarch64 binary without cross-compiling anything:

```console
$ cat > .env <<'ENV'
POSTGRES_PASSWORD=<a long random string>
KASL_AGENTS=employee@example.com:<the agent's token>
ENV
$ chmod 600 .env
$ docker compose -f docker-compose.prod.yml up -d --build
```

The database publishes no port — only the server reaches it, over the compose
network — and the server runs as an unprivileged user. The first build takes a
while on a small machine (about fifteen minutes on a Pi 4); later ones reuse
the cached layers.

This is a stand, not a supported deployment: backups, restore and an install
guide come with the deployment milestone.

## Configuration

Everything comes from the environment:

| Variable | Meaning | Default |
| --- | --- | --- |
| `DATABASE_URL` | PostgreSQL connection string | required |
| `KASL_SERVER_ADDR` | Address the HTTP server binds to | `0.0.0.0:8080` |
| `KASL_AGENTS` | Agents to provision on startup, as `email:token` pairs separated by commas | none |
| `RUST_LOG` | Log filter (tracing syntax) | `kasl_server=info,tower_http=info` |

Database migrations are embedded in the binary and applied on startup.

`KASL_AGENTS` is how the first agents get in while the admin UI does not exist
yet: each entry becomes an employee and an agent holding that token's hash.
Re-running with a changed token rotates it and revokes the old one. Tokens are
secrets — pass them through your deployment's secret store, not a committed
file — and the variable stops being the way in once tokens are issued from the
UI.

## What it will do

- Ingest work-time data from kasl agents: workdays, pauses, tasks, reports *(days, backfill and history import: done)*
- Manager dashboards: who is working right now, hours per person, trends over time
- Personal pages: every employee sees their own history
- Roles: admin, manager, employee
- Self-hosted: a single binary (Docker image planned) plus PostgreSQL — your data stays on your infrastructure

## Stack

Rust REST API (axum) + PostgreSQL (sqlx), React single-page app for the web UI. Architectural decisions are recorded in [docs/adr/](https://github.com/lacodda/kasl-server/tree/main/docs/adr).

## License

[MIT](https://github.com/lacodda/kasl-server/blob/main/LICENSE)
