# Changelog

All notable changes to this project are documented in this file.

## [0.2.0] - 2026-08-14

### CI
- Restore the publish tag trigger
- Run the schema tests against a real PostgreSQL

### Documentation
- Record the schema and how it differs from the agent

### Features
- Add the core schema for people, days, tasks and reports
- Log the schema version on startup and free port 5432
## [0.1.0] - 2026-08-13

### CI
- Add the tag-driven release and crates.io publish workflows
- Hold the publish tag trigger until trusted publishing exists

### Documentation
- Adopt the kilna frontend stack
- Document the v0.1.0 foundation with a live transcript
- Cut 0.1.0

### Features
- Bootstrap the axum server with /health on PostgreSQL

### Testing
- Guard README consistency before release
