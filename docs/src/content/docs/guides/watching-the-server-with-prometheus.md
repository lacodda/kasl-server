---
title: Watching the server with Prometheus
description: A metrics token, a scrape job, the alerts worth having and the panels that answer "is the server all right". Ten minutes if Prometheus is already running.
---

The server describes its own health at `/metrics` - the figures are in
[server metrics](/kasl-server/reference/server-metrics/). This guide points a
Prometheus you already run at it, adds the alerts that catch the failures a
team notices last, and draws the panels worth a glance.

## 1. Issue a metrics token

Sign in as an administrator. Open **Team**, then **Export → Read from a
dashboard tool…**, choose **Server metrics**, name the token after where it is
going (`Prometheus`) and **Issue a token**. Copy it now: it is shown once.

Only an administrator's token reads the metrics, and only while they are one.
If the account is ever made a manager, issue a new token from an
administrator's account and revoke the old one from the same screen.

## 2. Add a scrape job

Save the token in a file Prometheus can read - not in `prometheus.yml`, which
tends to end up in a repository:

```console
$ printf '%s' 'kasl_read_3b7e...' > /etc/prometheus/kasl-server.token
$ chmod 600 /etc/prometheus/kasl-server.token
```

Then the job, in `prometheus.yml`:

```yaml
scrape_configs:
  - job_name: kasl-server
    scheme: https
    authorization:
      type: Bearer
      credentials_file: /etc/prometheus/kasl-server.token
    static_configs:
      - targets: ["kasl.example.com"]
```

`/metrics` is the default path, so there is nothing more to say. Reload
Prometheus; under **Status → Targets** the job turns `UP` within one scrape.
`401` there means the token was mistyped or revoked; `403` means it is an hours
token, or its owner is no longer an administrator.

On a server reached over plain `http` inside a network - a home server, a
compose stack - use `scheme: http` and the address and port the server listens
on (`kasl-server:8080` from a container on the same compose network).

## 3. Alerts worth having

A rule file, `kasl-server.rules.yml`, listed under `rule_files:`:

```yaml
groups:
  - name: kasl-server
    rules:
      # Nothing answers, or the database is down - the token is checked
      # against it, so a dead database refuses the scrape too. /health
      # says which.
      - alert: KaslServerDown
        expr: up{job="kasl-server"} == 0
        for: 5m

      # The server is failing requests it should be answering.
      - alert: KaslServerErrors
        expr: sum(rate(kasl_http_requests_total{job="kasl-server", status=~"5.."}[10m])) > 0
        for: 10m

      # Agents' uploads are being refused. A few 4xx are an agent with a bad
      # day in its backlog; a stream of them is an agent that cannot deliver.
      - alert: KaslUploadsRefused
        expr: sum(rate(kasl_http_requests_total{job="kasl-server", route=~"/api/v1/days.*", status=~"4.."}[1h])) > 0.01
        for: 1h

      # The alert sweep runs every five minutes. Twenty minutes without one
      # finishing means nobody is being told about silent agents or long days.
      - alert: KaslSweepStalled
        expr: time() - kasl_background_last_success_timestamp_seconds{job="kasl-server", task="alert_sweep"} > 1200

      # The dispatcher runs every few seconds.
      - alert: KaslDispatcherStalled
        expr: time() - kasl_background_last_success_timestamp_seconds{job="kasl-server", task="webhook_dispatch"} > 300

      # A chat has not been reachable for an hour. Each delivery is retried
      # for about a day before it is abandoned.
      - alert: KaslWebhookBacklog
        expr: kasl_webhook_oldest_pending_age_seconds{job="kasl-server"} > 3600

      # Something was given up on: the receiver refused it, or a day of
      # retries ran out. The webhooks screen says why.
      - alert: KaslWebhookAbandoned
        expr: delta(kasl_webhook_deliveries{job="kasl-server", state="abandoned"}[1h]) > 0

      # Every connection is taken: requests are waiting for the pool.
      - alert: KaslPoolExhausted
        expr: sum(kasl_database_connections{job="kasl-server", state="busy"}) >= max(kasl_database_connections_max{job="kasl-server"})
        for: 5m
```

The thresholds are a start, not a law. A team in one timezone that stops at
night can raise `KaslUploadsRefused`'s window; one with agents across the
world will see uploads every hour of the day.

## 4. Panels

In Grafana, with Prometheus as the data source:

| Panel | Query |
| --- | --- |
| Requests per second, by status | `sum by (status) (rate(kasl_http_requests_total{job="kasl-server"}[5m]))` |
| Slowest routes, 95th percentile | `topk(5, histogram_quantile(0.95, sum by (le, route) (rate(kasl_http_request_duration_seconds_bucket{job="kasl-server"}[5m]))))` |
| Agents' pulses per minute | `sum(rate(kasl_http_requests_total{job="kasl-server", route="/api/v1/agent/heartbeat"}[5m])) * 60` |
| Pool in use | `kasl_database_connections{job="kasl-server", state="busy"}` against `kasl_database_connections_max` |
| Memory | `process_resident_memory_bytes{job="kasl-server"}` |
| Uptime | `time() - process_start_time_seconds{job="kasl-server"}` |
| Version | `kasl_build_info{job="kasl-server"}`, legend `{{version}}` |

The pulses per minute are how many agents are on: each sends one a minute
while its owner works. A cliff in the middle of a working day is the network
between them and the server, not the team going home at once.

## Grafana alone

Grafana Cloud's agent, Grafana Alloy and VictoriaMetrics' `vmagent` all scrape
the same way: a bearer token from a file, the target, the default path. The
names and queries above are the same in each.
