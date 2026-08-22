# Changelog

All notable changes to this project are documented in this file.

## [0.9.0] - 2026-08-22

### Documentation
- Describe the audit log and record what it does not keep

### Features
- Record who did what, and let an administrator read it

## [0.8.0] - 2026-08-21

### Breaking Changes
- For a manager, GET /api/v1/users now returns only the departments they run plus themselves, where it previously returned the whole company. Administrators and agents are unaffected. See ADR 0009 for the migration path: none is needed for a client that does not assume a company-wide list.

### Documentation
- Describe departments and record the visibility rules
- Cut 0.8.0, and stop hiding breaking commits from their group

### Features
- Give a manager a boundary to be in charge of

## [0.7.0] - 2026-08-20

### Documentation
- Describe managing the team, and record what a manager may do
- Cut 0.7.0

### Features
- Manage people and issue agent tokens

## [0.6.0] - 2026-08-19

### Documentation
- Describe signing in, and record why sessions are rows
- Cut 0.6.0

### Features
- Sign people in with server-side sessions

## [0.5.0] - 2026-08-17

### CI
- Run the whole suite against PostgreSQL, not a list of test files

### Documentation
- Document the import and refresh the transcript for 0.5.0
- Cut 0.5.0

### Features
- Bring an employee's local kasl history onto the server
- Bound an import by date, and document the whole thing

## [0.4.0] - 2026-08-17

### Documentation
- Describe backfill, deletions and which failures to retry
- Cut 0.4.0

### Features
- Let an agent declare a day's task list complete
- Accept a backlog of days on POST /api/v1/days/batch

## [0.3.1] - 2026-08-14

### Documentation
- Explain how to run a stand
- Refresh the transcript for 0.3.1
- Cut 0.3.1
- Render breaking changes as their own section

### Features
- Containerize the server and stage what a stand needs

## [0.3.0] - 2026-08-14

### CI
- Run the ingest tests against PostgreSQL too

### Documentation
- Match the published 0.2.0 release notes
- Document the ingest endpoint and the rules behind it

### Features
- Authenticate agents by bearer token
- Accept days from agents on POST /api/v1/days

### Refactoring
- Expose the server as a library

## [0.2.0] - 2026-08-14

### CI
- Restore the publish tag trigger
- Run the schema tests against a real PostgreSQL

### Documentation
- Record the schema and how it differs from the agent
- Cut 0.2.0
- Separate releases in the generated file
- Drop the forward reference to an unwritten record

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

