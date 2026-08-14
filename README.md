<p align="center"><img src="https://github.com/lacodda/kasl-server/raw/main/assets/banner.svg" alt="kasl-server - team server for kasl" width="720"></p>

# kasl-server

Team server for [kasl](https://github.com/lacodda/kasl). Employees run kasl on their machines; the agents send work-time data to the server. Managers get dashboards, charts, and reports across the whole team; every employee gets a personal page.

> **Status: pre-alpha.** v0.2.0 adds the core schema — people, agents, workdays, pauses, tasks, tags and reports — on top of the v0.1.0 foundation (axum on PostgreSQL, `/health`, structured logs, embedded migrations, release pipeline). The ingest API that fills those tables is the next milestone; nothing to deploy for real use yet.

## Try it

Requires Rust and Docker.

```console
$ git clone https://github.com/lacodda/kasl-server && cd kasl-server
$ docker compose up -d db
$ DATABASE_URL=postgres://kasl:kasl@localhost:5433/kasl cargo run
2026-08-14T17:19:24.126177Z  INFO kasl_server: database schema is up to date version=20260814000001
2026-08-14T17:19:24.126604Z  INFO kasl_server: kasl-server listening version="0.2.0" addr=0.0.0.0:8080

$ curl http://127.0.0.1:8080/health
{"database":"ok","status":"ok","version":"0.2.0"}
```

The dev database listens on 5433, leaving a PostgreSQL you may already run on
5432 alone; override with `KASL_DB_PORT`.

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

## Configuration

Everything comes from the environment:

| Variable | Meaning | Default |
| --- | --- | --- |
| `DATABASE_URL` | PostgreSQL connection string | required |
| `KASL_SERVER_ADDR` | Address the HTTP server binds to | `0.0.0.0:8080` |
| `RUST_LOG` | Log filter (tracing syntax) | `kasl_server=info,tower_http=info` |

Database migrations are embedded in the binary and applied on startup.

## What it will do

- Ingest work-time data from kasl agents: workdays, pauses, tasks, reports
- Manager dashboards: who is working right now, hours per person, trends over time
- Personal pages: every employee sees their own history
- Roles: admin, manager, employee
- Self-hosted: a single binary (Docker image planned) plus PostgreSQL — your data stays on your infrastructure

## Stack

Rust REST API (axum) + PostgreSQL (sqlx), React single-page app for the web UI. Architectural decisions are recorded in [docs/adr/](https://github.com/lacodda/kasl-server/tree/main/docs/adr).

## License

[MIT](https://github.com/lacodda/kasl-server/blob/main/LICENSE)
