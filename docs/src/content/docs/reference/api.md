---
title: The API
description: The ingest contract - uploading a day or a backlog, and telling agents apart.
---

`/api/v1` from the first endpoint: agents update on their own schedule, so a
path keeps meaning what it meant when the agent calling it shipped.

**`POST /api/v1/days`** — upload one day. Requires `Authorization: Bearer
<token>`. The body is the workday with its pauses and tasks; `ended_at` is
absent while the day is still running, and so is a pause's, and `tasks` may be
empty.

Two properties are worth knowing before writing a client:

- **Every instant needs a UTC offset**, and the day carries its own `date`.
  `2026-08-14T09:12:00` without an offset is refused (422): one team's hours
  have to stay comparable across time zones, and which calendar day work
  belongs to is the agent's call, not a value derived on the server.
- **The last upload wins.** Re-sending a day replaces what is stored, so a
  correction made in kasl lands and a retry after a lost connection is safe -
  the same payload twice leaves the same rows. Pauses are replaced as a set;
  tasks are matched on `agent_task_id`, so a task carried into the next day
  moves rather than multiplying.
- **Deleting a task takes one word.** Send `"tasks_are_complete": true` and the
  date's tasks the payload omits are deleted, which is how a task the employee
  removed in kasl disappears here too. Other dates are untouched. Leave the flag
  out - as agents written before it did - and nothing is ever deleted.
- **A day off is a `kind`.** Send `"kind": "vacation"`, `"sick"` or `"day_off"`
  for a day the person was away; the norm then asks nothing of that date
  instead of reporting it as hours missing. The field is optional and defaults
  to `"work"`, so an agent written before it exists is read exactly as it was.
  See [the calendar and the norm](/kasl-server/reference/the-calendar-and-the-norm/).

**`POST /api/v1/days/batch`** — upload a backlog. The body is `{"days": [...]}`
with the same day objects, and the answer reports each one:

```json
{"accepted": 2, "rejected": 1, "results": [
  {"status": "accepted", "date": "2026-08-10", "kind": "work", "workday_id": "...", "pauses": 1, "tasks": 3, "deleted_tasks": 0, "privacy_level": "full"},
  {"status": "rejected", "date": "2026-08-11", "error": "ended_at is before started_at"},
  {"status": "accepted", "date": "2026-08-12", "kind": "vacation", "workday_id": "...", "pauses": 0, "tasks": 0, "deleted_tasks": 0, "privacy_level": "full"}
]}
```

Each day is written on its own, so one the server will never accept does not
block the rest - an agent that could not deliver *any* of its backlog because of
a single bad row would retry the same request forever. The batch carries at most
`KASL_MAX_BATCH_DAYS` days (31) and the body at most `KASL_MAX_BODY_BYTES`
(4 MiB); past either, `413`.

**Which failures are worth retrying.** `4xx` means the payload will not be
accepted as sent, however many times it is tried - fix it or drop it. `5xx`
means the fault is on this side; send it again later. A batch that answers `5xx`
stopped partway: the days already accepted are stored, and re-sending them is
safe because the last upload wins.

**`GET /api/v1/agent/whoami`** — whose token this is. An agent token is opaque
to the machine holding it, so a token pasted from the wrong place would file
that machine's days under a colleague's name without a word. This answers the
question at connect time, while a person is watching:

```console
$ curl -H "Authorization: Bearer $KASL_TOKEN" http://127.0.0.1:8080/api/v1/agent/whoami
{"user_name":"kirill","agent_name":"laptop","api_version":"v1","server_version":"0.14.1"}
```

Refused with `401` for a token that is unknown, revoked, or belongs to a
deactivated account - the same answer the upload routes give, because a token
that cannot write must not be able to read a name off the installation either.

**Every accepted day reports the privacy level that applied**, and a
`discarded` object when that level left something out. See
[what the server stores about you](/kasl-server/concepts/what-the-server-stores-about-you/): an agent
should never report a break as recorded by a server that did not store it.

A malformed day is refused with `400` and a reason naming the field
(`{"error":"tasks[0]: completeness must be between 0 and 100"}`); an
unrecognized, revoked or deactivated token gets `401`. The reasoning behind all
of this is in [ADR 0004](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0004-the-ingest-contract.md)
and [ADR 0005](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0005-deletions-and-backfill.md).
