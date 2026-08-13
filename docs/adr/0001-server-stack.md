# 0001. Server stack: Rust REST API, PostgreSQL, React SPA

Date: 2026-08-12
Status: Accepted

## Context

kasl is a local-first CLI time tracker: every employee's data lives in SQLite on their own machine. A team needs a central place — managers want dashboards and reports across people, employees want a personal page with their own history. The lacodda product line standardizes on Rust and a shared production pipeline (CI matrix, git-cliff changelogs, tag-driven releases), and decisions proven in one project are carried into the next.

## Decision

- REST API server in Rust on **axum** (tokio runtime) — same language and toolchain as the rest of the line.
- **PostgreSQL** via sqlx with embedded migrations. Concurrent writes from many agents and cross-employee reporting queries are exactly what SQLite is not for.
- **React SPA** in `frontend/`, served by the server as static files. One monorepo: an API change and the UI that consumes it land in one commit.
- The API is **versioned from the first endpoint** (`/api/v1/...`). kasl agents are external clients that update on their own schedule; the contract must not break silently.

## Consequences

- Deployment is a single binary plus PostgreSQL; a Docker image is planned for the deployment milestone.
- The Node toolchain enters the repo only at the web-UI milestone; CI grows a frontend job then.
- Breaking API changes require a new API version path and a documented migration for agents.
