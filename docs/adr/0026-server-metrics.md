# 0026. Server metrics for Prometheus

Date: 2026-10-26

Status: Accepted

## Context

Until now the server said one thing about its own health: `/health`, a JSON
answer to "is the process up, and can it reach the database". That is enough
for a container's restart policy and nothing more. An operator running the
server for a team has the questions a liveness probe cannot answer: are the
requests getting slower, are agents' uploads failing, is the alert sweep still
running, is a chat hook refusing every message, is the pool out of connections.

Every self-hosted service answers those the same way - a `/metrics` endpoint in
the Prometheus text format, scraped by whatever the operator already runs - and
an operator who has one such service on a dashboard expects the next to drop in
beside it. Three questions stood between that and this server.

**Who may read it.** The figures name no person, but they are not public: the
request rate by route says how many agents report and when, and the demo stand
is on the open internet. Prometheus can send a bearer token; it cannot sign in.

**Where the figures come from.** Some exist only in the process - how many
requests, how long each took, whether the last sweep finished. Others are
already rows - the webhook queue, with every delivery's outcome (ADR 0019).

**What holds still.** An alert rule names a series. A series renamed under it
does not make the rule fail; it makes it silent, which reads as "all is well".

## Decision

**`/metrics`, read with a read token of scope `metrics`.** The read tokens of
ADR 0025 gain a second scope. A token of scope `metrics` is an administrator's:
the figures describe the installation, and the installation is theirs. The
rule of who may hold a scope lives on the scope (`ReadScope::held_by`), and
both issuing a token and every request made with it ask it - so an
administrator made a manager stops reading the metrics at once, as a manager
made an employee stops reading the hours. The route takes `ReadToken<Metrics>`;
an hours token is refused at it with `403`, and a metrics token at the BI API.

The other ways were weighed. A second listener on its own port, without a
token, is the Kubernetes habit; here it would be a setting every installation
has to get right - bind it to the inside, never publish it - and the demo shows
what one wrong compose file does. An open `/metrics` publishes the request
pattern of a team to anyone. A token is what the server already issues,
revokes, audits and lists.

**A read token module, not a BI one.** The tokens moved from `bi::token` to
`read_token`: they now open two doors, and only one of them is the BI API.

**Two kinds of figure, each from where it lives.**

* What happened in this process is counted as it happens: requests by method,
  route pattern and status; their durations as a histogram; the runs of the
  two timed tasks (the alert sweep, the webhook dispatcher) by outcome, and the
  moment each last completed. These start from zero with the process, which
  Prometheus reads as a counter reset.
* What the database holds is read from it at the scrape, never kept in memory:
  deliveries per configured destination by state (pending, delivered,
  abandoned), and how long the oldest pending one has waited. The query is the
  one the webhooks screen reads (`webhooks::queues`), so the screen and the
  graph cannot disagree.

Plus the figures every client exports under their standard names: the build's
version as `kasl_build_info`, `process_start_time_seconds`, and on Linux the
process's CPU, memory, threads and file descriptors from `/proc`. Not on other
systems: the server runs on Linux, and a second source per platform is a second
place to be wrong.

**A route by its pattern, never its path.** A request is counted under the
pattern of the route that answered - `/api/v1/users/{id}/days` - so no series
names a person, and a scanner asking for a thousand made-up paths makes one
series (`unmatched` under `/api`, `web` elsewhere), not a thousand. A method
outside the standard set is `OTHER` for the same reason.

**A scrape never fails over the database.** The database's figures are read
under a two-second limit; past it, the scrape answers without them. Prometheus
abandons a scrape after ten seconds by default, and an abandoned scrape loses
the process's figures too - the ones that say why the database is slow. There
is no `kasl_database_up`: the token is checked against the database, so with
the database down the scrape is refused, and Prometheus's own `up` series and
`/health` say so.

**A task's last success starts at the process's start.** A task stuck in its
first run never completes, and a series that does not exist cannot grow old
enough to alert on. Before its first success, a task's
`kasl_background_last_success_timestamp_seconds` is the moment the server
started.

**OpenMetrics, the format Prometheus asks for.** Served as
`application/openmetrics-text`, through the `prometheus-client` crate - the
Prometheus project's own client, which writes the format whole (units, the
closing `# EOF`) and types each family's labels as a struct, so a sample with a
missing or misspelled label does not compile.

**The names are a contract.** Within this major version a figure, a label or a
label's value is only ever added. The published list is copied into the tests,
which fail on a rename, and the reference page is checked against it.

## Consequences

- A metrics token reads nobody's data, so the privacy manifest's
  `read_elsewhere` does not list it. The manifest asks the scope what a token
  reads, in a match the compiler holds exhaustive: the finance milestone's
  `payroll` scope cannot be added without saying what it reads about a person.
- With a team of three, the rate of heartbeats says how many agents are on.
  That is the administrator's to know already - the team screen says who - and
  the reason the token is theirs alone.
- Issuing a token now names its scope (`{"name", "scope"}`); there is no
  default, because a default is the scope somebody did not choose.
- A scrape costs four small queries: the token, its last-used moment, a round
  trip, the queue. At Prometheus's usual 15 to 60 seconds that is nothing.
- No figure counts uploads or days by outcome. The request counters by route
  and status already say whether ingest succeeds; a second count of the same
  requests would be a second number to reconcile.
