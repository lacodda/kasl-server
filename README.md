<p align="center"><img src="https://github.com/lacodda/kasl-server/raw/main/assets/banner.svg" alt="kasl-server - team server for kasl" width="720"></p>

> Employees run kasl on their machines; managers get dashboards, charts, and reports across the whole team - and every employee gets a personal page.

<p align="center">
  <a href="https://crates.io/crates/kasl-server"><img src="https://img.shields.io/crates/v/kasl-server?style=flat-square" alt="crates.io"></a>
  <a href="https://github.com/lacodda/kasl-server/actions"><img src="https://img.shields.io/github/actions/workflow/status/lacodda/kasl-server/ci.yml?style=flat-square" alt="CI"></a>
  <a href="https://github.com/lacodda/kasl-server/blob/main/LICENSE"><img src="https://img.shields.io/github/license/lacodda/kasl-server?style=flat-square" alt="License"></a>
</p>

## Why

A team using [kasl](https://github.com/lacodda/kasl) has work-time data
scattered across everyone's own machine: each agent knows its owner's hours,
and nobody else's. kasl-server is where that data converges - agents push
their days to it, and it turns the sum into a manager's dashboard and each
employee's own page.

The door for agents is built to survive a bad connection: a day at a time or a
whole backlog in one request, and a task deleted in kasl disappears here too.
What the server keeps about a person is a policy it enforces and can recite,
not a claim in a document - an employee can ask it, and an administrator can
narrow it. kasl has been delivering days on its own since its v1.8, backlog
included, so the loop is closed end to end.

## A day in the life

```console
$ docker compose up -d
$ curl http://127.0.0.1:8080/health
{"database":"ok","demo":false,"status":"ok","version":"0.21.1"}

# An agent back from three days offline - the middle day is impossible, and
# the others land anyway.
$ curl -X POST http://127.0.0.1:8080/api/v1/days/batch \
    -H "Authorization: Bearer agent-token" -H "Content-Type: application/json" \
    -d '{"days":[...]}'
{"accepted":2,"rejected":1,"results":[
  {"status":"accepted","date":"2026-08-15","workday_id":"82ca500d-...","pauses":0,"tasks":1,"deleted_tasks":0},
  {"status":"rejected","date":"2026-08-16","error":"ended_at is before started_at"},
  {"status":"accepted","date":"2026-08-17","workday_id":"2ea46d40-...","pauses":0,"tasks":1,"deleted_tasks":0}]}
```

The web UI is served by the same binary on the same port - open
`http://127.0.0.1:8080` and sign in. Nothing to look at yet?
`KASL_DEMO=true docker compose up -d` seeds a fictional team so the dashboards
can be seen before an agent is installed anywhere.

## What you get

- **A door that survives a bad connection.** One day or a backlog in one
  request; each day is written on its own, so one bad row does not block the
  rest.
- **Dashboards for a manager.** The team's hours, a drill-down into anyone's
  days, a live column of who is working right now, the month as a heatmap,
  and signals naming who is worth a look.
- **A personal page for every employee.** Their own week, drawn as a timeline
  of work and the pauses in it, against the hours it asked for.
- **A production calendar and a norm.** Holidays, shortened eves and
  transferred weekends; a full day per installation and a share of it per
  person, so half time reads as half time rather than as half-hearted.
- **A privacy policy the server enforces.** Three levels of detail, applied on
  the way in - a field a level excludes never reaches the database.
- **Roles and departments.** Admins, managers and employees, with visibility
  rules that keep a manager inside their own department.
- **An audit log of everything that changes people, departments or tokens.**
- **Self-hosted, one binary.** API and web UI in one file, plus PostgreSQL -
  your data stays on your infrastructure.

## Install

Docker and nothing else - no Rust, no Node, no build:

```console
$ curl -o docker-compose.yml \
    https://raw.githubusercontent.com/lacodda/kasl-server/main/docker-compose.install.yml
$ printf 'POSTGRES_PASSWORD=%s\n' "$(openssl rand -base64 24)" > .env
$ docker compose up -d
```

The image is `ghcr.io/lacodda/kasl-server`, built for amd64 and arm64, so the
same compose file works on a laptop and on a Raspberry Pi. Full instructions,
including backups and the administrator account: [Installing it](https://lacodda.github.io/kasl-server/guides/installing-it/).

## Status

v0.22.0, in daily use on a small team's own stand. The ingest contract,
dashboards, roles, departments, the audit log, the privacy manifest, the
production calendar, alerts that arrive without being asked for, and the web
UI - including a phone layout - all hold end to end. What landed in each
version:
[CHANGELOG](https://github.com/lacodda/kasl-server/blob/main/CHANGELOG.md).

## Documentation

**[lacodda.github.io/kasl-server](https://lacodda.github.io/kasl-server/)** -
the API, the data model, the privacy manifest and the decisions behind them.
Architecture decision records are in [`docs/adr/`](https://github.com/lacodda/kasl-server/tree/main/docs/adr).

Building it yourself: [CONTRIBUTING.md](https://github.com/lacodda/kasl-server/blob/main/CONTRIBUTING.md).

## License

MIT (c) [Kirill Lakhtachev](https://lacodda.com)
