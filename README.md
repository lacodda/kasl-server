<p align="center"><img src="https://github.com/lacodda/kasl-server/raw/main/assets/banner.svg" alt="kasl-server - team server for kasl" width="720"></p>

# kasl-server

Team server for [kasl](https://github.com/lacodda/kasl). Employees run kasl on their machines; the agents send work-time data to the server. Managers get dashboards, charts, and reports across the whole team; every employee gets a personal page.

> **Status: pre-alpha.** v0.1.0 is the foundation: an axum server on PostgreSQL with `/health`, structured logs, embedded migrations, and a release pipeline. The ingest API for kasl agents is the next milestone; nothing to deploy for real use yet.

## Try it

Requires Rust and Docker.

```console
$ git clone https://github.com/lacodda/kasl-server && cd kasl-server
$ docker compose up -d db
$ DATABASE_URL=postgres://kasl:kasl@localhost:5432/kasl cargo run
2026-08-13T19:08:20.841963Z  INFO kasl_server: kasl-server listening version="0.1.0" addr=0.0.0.0:8080

$ curl http://127.0.0.1:8080/health
{"database":"ok","status":"ok","version":"0.1.0"}
```

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
