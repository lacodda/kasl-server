<p align="center"><img src="https://github.com/lacodda/kasl-server/raw/main/assets/banner.svg" alt="kasl-server - team server for kasl" width="720"></p>

# kasl-server

Team server for [kasl](https://github.com/lacodda/kasl). Employees run kasl on their machines; the agents send work-time data to the server. Managers get dashboards, charts, and reports across the whole team; every employee gets a personal page.

> **Status: pre-alpha.** v0.3.0 opens the door for kasl agents: `POST /api/v1/days` accepts a day at a time, authenticated by a bearer token. The tables are filled; nothing reads them back yet — dashboards and the personal page are the milestones after next, and there is still nothing to deploy for real use.

## Try it

Requires Rust and Docker.

```console
$ git clone https://github.com/lacodda/kasl-server && cd kasl-server
$ docker compose up -d db
$ export DATABASE_URL=postgres://kasl:kasl@localhost:5433/kasl
$ export KASL_AGENTS=employee@example.com:agent-token
$ cargo run
2026-08-14T21:28:05.552455Z  INFO kasl_server: database schema is up to date version=20260814000001
2026-08-14T21:28:05.627689Z  INFO kasl_server::provision: provisioned agents from KASL_AGENTS agents=1
2026-08-14T21:28:05.628303Z  INFO kasl_server: kasl-server listening version="0.3.1" addr=0.0.0.0:8080

$ curl http://127.0.0.1:8080/health
{"database":"ok","status":"ok","version":"0.3.1"}

$ curl -X POST http://127.0.0.1:8080/api/v1/days \
    -H "Authorization: Bearer agent-token" -H "Content-Type: application/json" \
    -d '{"date":"2026-08-14",
         "started_at":"2026-08-14T09:12:00-03:00",
         "ended_at":"2026-08-14T18:31:00-03:00",
         "pauses":[{"started_at":"2026-08-14T13:02:00-03:00","ended_at":"2026-08-14T14:05:00-03:00","duration_seconds":3780,"manual":true,"reason":"lunch"}],
         "tasks":[{"agent_task_id":1,"recorded_at":"2026-08-14T18:28:00-03:00","name":"Ingest API v1","completeness":100}]}'
{"workday_id":"6d593db2-dce9-47f5-95aa-6ef28cdbda96","date":"2026-08-14","pauses":1,"tasks":1}
```

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

A malformed day is refused with `400` and a reason naming the field
(`{"error":"tasks[0]: completeness must be between 0 and 100"}`); an
unrecognized, revoked or deactivated token gets `401`. The reasoning behind all
of this is in [ADR 0004](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0004-the-ingest-contract.md).

## The data model

Migrations live in `migrations/` and are applied on startup. The shape follows
kasl's own model, so a reader who knows the agent recognizes it:

| Table | Holds |
| --- | --- |
| `users` | People and their role: admin, manager, employee |
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

- Ingest work-time data from kasl agents: workdays, pauses, tasks, reports *(days: done)*
- Manager dashboards: who is working right now, hours per person, trends over time
- Personal pages: every employee sees their own history
- Roles: admin, manager, employee
- Self-hosted: a single binary (Docker image planned) plus PostgreSQL — your data stays on your infrastructure

## Stack

Rust REST API (axum) + PostgreSQL (sqlx), React single-page app for the web UI. Architectural decisions are recorded in [docs/adr/](https://github.com/lacodda/kasl-server/tree/main/docs/adr).

## License

[MIT](https://github.com/lacodda/kasl-server/blob/main/LICENSE)
