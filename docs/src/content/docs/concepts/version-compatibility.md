---
title: Which kasl works with which server
description: The floor is set by whoami, not by the ingest endpoints - and a newer server is always safe for an older agent.
---

The two products ship on their own schedules, so the question "will this agent
talk to that server" has to have an answer that is not "try it". Each kasl
feature below names the endpoint it calls and the server version that first
answered it.

| kasl | What it does | Endpoint | Needs server |
| --- | --- | --- | --- |
| v1.7.0 | `kasl server connect`, `status` | `GET /health`, `GET /api/v1/agent/whoami` | **0.14.1** |
| v1.8.0 | `kasl server push` — one day | `POST /api/v1/days` | **0.14.1** |
| v1.9.0 | `queue`, `flush`, `backfill` — a backlog in one request | `POST /api/v1/days/batch` | **0.14.1** |
| v1.13.0 | `manifest` — what the server keeps about you | `GET /api/v1/privacy/agent` | **0.14.1** |

The floor is 0.14.1 across the board, and it is set by `whoami` rather than by
uploading: ingest has answered since 0.3.0 and batch since 0.4.0, but every
version of kasl that can send anything checks whose token it holds first, and
refuses to store a connection the check did not pass. An older server is not
half-usable — it is unreachable from a released agent.

**What an agent sees on a server that is too old.** The `/api` routes answer an
unknown path with `{"error":"no such endpoint"}` and `404` rather than letting
the web UI's fallback return a page, so kasl reports a refusal naming the
endpoint instead of hanging or reading HTML as success. A `404` is classified as
rejected, not retryable: the agent does not queue days against a server that
will never take them.

**A newer server is always safe.** `/api/v1` is a promise: a path keeps its
meaning for as long as agents call it, and anything that would change that
meaning becomes `/api/v2` with a migration written for agents. Everything the
server has added since — departments, the audit log, the privacy manifest,
signals, the heatmap, the webhooks — is read by people through the web UI and
changes nothing an agent sends. The manifest an agent reads grew a
`sent_elsewhere` list in 0.23.0; an agent that does not know the field shows
the rest of the manifest exactly as before. The additions an agent could use have no client yet, each paired with a kasl
version that has not shipped: `POST /api/v1/agent/heartbeat` (0.17.0), and the
[notifications](/kasl-server/reference/what-you-are-told/) it counts -
`GET /api/v1/agent/notifications` with its `ack` and `read` (0.24.0), for kasl
v3.3. The pulse's answer grew a `notifications` field for them; an agent that
does not know it ignores it.

**Releasing a contract change.** A change to the shape of what agents send or
receive lands in both products before either is tagged: the server's endpoint
and the docs here, then kasl's client and this table's new row. The order
matters — the server goes first and stays backward compatible, because an agent
updates when its owner decides to, and a server that requires a version of kasl
nobody is running yet is a server nobody can send to.
