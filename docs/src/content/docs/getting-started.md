---
title: Getting Started
description: Bring up kasl-server locally, see the API and the web UI answer, and feed it a backlog of days.
---

Requires Rust and Docker.

```console
$ git clone https://github.com/lacodda/kasl-server && cd kasl-server
$ docker compose up -d db
$ export DATABASE_URL=postgres://kasl:kasl@localhost:5433/kasl
$ export KASL_AGENTS=employee@example.com:agent-token
$ cargo run
2026-08-29T18:15:18.767453Z  INFO kasl_server: database schema is up to date version=20260830000001
2026-08-29T18:15:18.811231Z  INFO kasl_server::provision: provisioned agents from KASL_AGENTS agents=1

  An administrator account was created, because this installation had none:

      email:    admin@kasl.local
      password: ye4e9vgapwi8ptrh9kt9

  This is the only time it is shown. Sign in and change it.

2026-09-17T13:30:12.118204Z  INFO kasl_server: kasl-server listening version="0.20.0" addr=0.0.0.0:8080 max_batch_days=31 max_body_bytes=4194304

$ curl http://127.0.0.1:8080/health
{"database":"ok","demo":false,"status":"ok","version":"0.20.0"}

# The web UI is served by the same binary on the same port - open
# http://127.0.0.1:8080 and sign in.

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

## Where to go next

- **[Installing it](/kasl-server/guides/installing-it/)** - the Docker-only path for a real deployment.
- **[The demo](/kasl-server/guides/the-demo/)** - see the dashboards before installing an agent anywhere.
- **[The API](/kasl-server/reference/api/)** - the ingest contract agents talk to.
- **[The web UI](/kasl-server/guides/the-web-ui/)** - what a manager and an employee each see.
