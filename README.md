<p align="center"><img src="https://github.com/lacodda/kasl-server/raw/main/assets/banner.svg" alt="kasl-server - team server for kasl" width="720"></p>

# kasl-server

Team server for [kasl](https://github.com/lacodda/kasl). Employees run kasl on their machines; the agents send work-time data to the server. Managers get dashboards, charts, and reports across the whole team; every employee gets a personal page.

> **Status: pre-alpha.** The project is scaffolded; v0.1.0 (server skeleton on PostgreSQL) is in development. Nothing to install yet.

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
