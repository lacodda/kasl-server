---
title: Server metrics
description: GET /metrics - the server's own health for Prometheus. An administrator's read token of scope metrics, the OpenMetrics format, and names that stay put.
---

`/health` says whether the server is up. `/metrics` says how it is doing:
requests and how long they take, the timed tasks, the database pool, the
webhook queue and the process itself, in the format Prometheus scrapes.

```console
$ curl -H "Authorization: Bearer kasl_read_3b7e..." https://kasl.example.com/metrics
# HELP kasl_build_info The version of kasl-server answering; always 1.
# TYPE kasl_build_info gauge
kasl_build_info{version="0.30.0"} 1
...
# EOF
```

Setting up Prometheus and the alerts worth having is in
[watching the server with Prometheus](/kasl-server/guides/watching-the-server-with-prometheus/).

## The token

`/metrics` is read with a **read token of scope `metrics`** - the tokens of
[the BI API](/kasl-server/reference/the-bi-api/#read-tokens), issued for the
server rather than for the hours. Only an administrator issues one, and it
reads only while its owner is still an administrator: make them a manager and
it answers `403`, deactivate them and it answers `401`.

Issue it in the web UI - **Export → Read from a dashboard tool**, then
**Server metrics** - or signed in, with
`POST /api/v1/read-tokens` and `{"name": "Prometheus", "scope": "metrics"}`.

The two scopes do not mix: an hours token is refused at `/metrics` with `403`,
and a metrics token at `/api/bi/v1`. A metrics token reads no person's data, so
[what the server stores about you](/kasl-server/concepts/what-the-server-stores-about-you/)
does not list it among the tools reading people's hours.

## The format

OpenMetrics text (`application/openmetrics-text; version=1.0.0`), the format
Prometheus asks for. It ends with `# EOF`, which is how Prometheus knows the
scrape arrived whole. Counters carry the `_total` suffix and figures in a unit
carry the unit in their name: `_seconds`, `_bytes`.

No figure names a person. A request is counted under the **pattern** of the
route that answered - `/api/v1/users/{id}/days`, never the id - and a
destination under the label the operator gave it.

## The figures

| Name | Type | Labels | What it is |
| --- | --- | --- | --- |
| `kasl_build_info` | gauge | `version` | Always `1`; the version answering. An upgrade shows as the label changing. |
| `kasl_http_requests_total` | counter | `method`, `route`, `status` | Requests answered. |
| `kasl_http_request_duration_seconds` | histogram | `method`, `route` | Time from a request arriving to its response's head. Buckets from 5 ms to 10 s. |
| `kasl_background_runs_total` | counter | `task`, `outcome` | Runs of the timed tasks, `outcome` `ok` or `error`. |
| `kasl_background_last_success_timestamp_seconds` | gauge | `task` | When each task last completed. Before its first success, the moment the server started - so a task stuck in its first run still ages. |
| `kasl_database_connections` | gauge | `state` | Open connections to PostgreSQL, `state` `idle` or `busy`. |
| `kasl_database_connections_max` | gauge | | The most the server opens. Busy at the maximum means requests queue for a connection. |
| `kasl_database_round_trip_seconds` | gauge | | How long PostgreSQL took to answer a trivial query, at this scrape. |
| `kasl_webhook_deliveries` | gauge | `destination`, `state` | Deliveries to each configured destination, `state` `pending`, `delivered` or `abandoned`. |
| `kasl_webhook_oldest_pending_age_seconds` | gauge | `destination` | How long the oldest delivery still waiting has waited; `0` when none waits. |
| `process_start_time_seconds` | gauge | | When the server started, in seconds since the Unix epoch. |
| `process_cpu_seconds_total` | counter | | CPU time, user and system. Linux only. |
| `process_resident_memory_bytes` | gauge | | Resident memory. Linux only. |
| `process_virtual_memory_bytes` | gauge | | Virtual memory. Linux only. |
| `process_threads` | gauge | | Threads. Linux only. |
| `process_open_fds` | gauge | | Open file descriptors. Linux only. |
| `process_max_fds` | gauge | | The limit on them; absent when there is none. Linux only. |

### Label values

- **`method`** - `GET`, `HEAD`, `POST`, `PUT`, `PATCH`, `DELETE`, `OPTIONS`, and
  `OTHER` for anything else a client sends.
- **`route`** - the pattern of the route that answered, as
  [the API reference](/kasl-server/reference/api/) writes it with its `/api/v1`
  prefix, and `/metrics` and `/health` themselves. A request no route claimed
  is `unmatched` under `/api` (the JSON `404`) and `web` anywhere else, where
  the web UI answers.
- **`status`** - the HTTP status, as a number.
- **`task`** - `alert_sweep`, the pass that raises and clears
  [alerts](/kasl-server/reference/what-needs-attention/) every five minutes, and
  `webhook_dispatch`, the pass that sends [webhooks](/kasl-server/reference/webhooks/).
- **`destination`** - the label from `KASL_WEBHOOK_<NAME>`, lower-cased, as the
  webhooks screen shows it. Never the address.

### Where each comes from

The request, task and process figures are this process's own and start from
zero when it does; Prometheus reads a restart as a counter reset. The webhook
figures are read from the database at each scrape, so they survive a restart
and match the webhooks screen.

The database's figures are read under a two-second limit. Past it, the scrape
answers without `kasl_database_round_trip_seconds` and the webhook figures
rather than not at all - and logs why. With PostgreSQL down, the token cannot
be checked and the scrape is refused: Prometheus's own `up` goes to `0`, and
[`/health`](/kasl-server/getting-started/) says it is the database.

## What stays the same

Within this major version a figure, a label or a label's value is only ever
added - never renamed, removed or given a different meaning. An alert rule
written against these names keeps meaning what it meant.
