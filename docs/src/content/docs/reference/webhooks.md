---
title: Webhooks
description: The events the server sends, the JSON a receiver gets, how to check its signature, and the two routes behind the Webhooks screen.
---

Destinations are declared in the environment - see
[Sending alerts to a chat](/kasl-server/guides/sending-alerts-to-a-chat/). This
page is for writing a receiver, and for the routes the web UI uses.

## Events

| Event | When |
| --- | --- |
| `alert.raised` | A sweep found a condition true that had no open alert |
| `alert.acknowledged` | A manager answered an alert ("I looked") |
| `alert.resolved` | The condition stopped being true - answered or not |
| `day.closed` | A day arrived finished for the first time, having ended within the last 24 hours |
| `test` | An administrator asked for one |

`day.closed` is sent once per day: re-sending a finished day, or correcting
when it ended, is not a second close. A backlog of days that ended long ago -
an agent back from a fortnight offline - is not announced at all.

## What a `json` destination receives

A `POST` with the event as the body:

```http
POST /kasl HTTP/1.1
Content-Type: application/json
User-Agent: kasl-server/0.23.0
X-Kasl-Event: alert.raised
X-Kasl-Event-Id: 5b3f2c1e-8d4a-8f1b-a2c3-9e8d7f6a5b4c
X-Kasl-Signature: t=1758614400,sha256=6f1e...c02b

{"version":1,
 "id":"5b3f2c1e-8d4a-8f1b-a2c3-9e8d7f6a5b4c",
 "event":"alert.raised",
 "occurred_at":"2026-09-23T08:00:00Z",
 "server_version":"0.23.0",
 "link":"https://kasl.example.com/team/0073460d-...",
 "person":{"id":"0073460d-...","name":"Jonas Petit","department":"Design"},
 "alert":{"id":"7f1c...","rule":"no_agent_data","observed_seconds":108000,
          "against_seconds":43200,"subject_date":null,"fired_at":"2026-09-23T08:00:00Z"}}
```

| Field | Present | Meaning |
| --- | --- | --- |
| `version` | always | The payload's shape. A change to what a field means is a new number |
| `id` | always | The same for every destination and every retry. Deduplicate on it |
| `event` | always | One of the events above |
| `occurred_at` | always | When the server noticed |
| `server_version` | always | The server that sent it |
| `link` | with `KASL_PUBLIC_URL` | Where to look |
| `person` | not on `test` | Who it is about: `id`, `name`, `department` (may be null) |
| `alert` | on `alert.*` | The alert as it fired: `rule`, `observed_seconds`, `against_seconds`, `subject_date`, `fired_at`. The figures never change after firing - a resolution carries the ones it was raised with |
| `by` | on `alert.acknowledged` | The name of whoever answered it |
| `day` | on `day.closed` | `date`, `kind` (`work`, `vacation`, `sick`, `day_off`), `started_at`, `ended_at`, `worked_seconds` - the span less its pauses |

The rules, and what their figures mean, are in
[What needs attention](/kasl-server/reference/what-needs-attention/).

**Answer `2xx` to accept.** A `4xx` other than `408` and `429` is taken as a
refusal and not sent again; anything else is retried, with `Retry-After` in
seconds honoured up to an hour. Delivery is **at least once**: a request that
timed out on this side may have arrived on yours, so the same `id` can come
twice.

## Checking the signature

`X-Kasl-Signature` is `t=<unix seconds>,sha256=<hex>`, where the hex is
HMAC-SHA256 of `<t>.<body>` under the destination's `secret=`. The timestamp is
inside what is signed, so a captured request cannot be replayed later under a
fresh one. Sign the raw body you received - not a re-serialized copy - and
refuse a `t` far from your own clock:

```python
import hashlib, hmac, time

def verify(secret: str, header: str, body: bytes, tolerance: int = 300) -> bool:
    parts = dict(item.split("=", 1) for item in header.split(","))
    timestamp, received = parts["t"], parts["sha256"]
    if abs(time.time() - int(timestamp)) > tolerance:
        return False
    expected = hmac.new(secret.encode(), f"{timestamp}.".encode() + body, hashlib.sha256).hexdigest()
    return hmac.compare_digest(expected, received)
```

## The chat kinds

Slack, Mattermost and Telegram get the same event as a message: who it is
about, the sentence the dashboard shows, and the link.

```text
Needs attention: Jonas Petit (Design)
Nothing from their agent for 30 h
Open in kasl-server
```

Slack and Mattermost receive `{"text": ...}` at the hook; Telegram receives
`sendMessage` with `parse_mode: HTML` through the bot. Names are escaped for
each markup, so a display name is shown as typed, never read as formatting.

## The routes behind the screen

Both are for administrators; anyone else gets `403`.

**`GET /api/v1/webhooks`** - the destinations and the last fifty deliveries.
No address is ever in the answer: a destination is shown by its label, its kind
and its host (or its chat id), which is enough to tell two apart and never
enough to post through one.

```json
{"links": true,
 "destinations": [
   {"name": "team", "kind": "slack", "target": "hooks.slack.com",
    "events": ["alert.raised", "alert.acknowledged", "alert.resolved"],
    "department": null, "department_exists": null,
    "pending": 0, "delivered": 41, "abandoned": 1,
    "last_delivered_at": "2026-09-23T08:00:04Z",
    "last_error": "the receiver answered 404 Not Found: no_service",
    "last_error_at": "2026-09-20T11:30:00Z"}],
 "recent": [
   {"id": "...", "event_id": "...", "destination": "team", "event": "alert.raised",
    "person": "Jonas Petit", "created_at": "2026-09-23T08:00:00Z", "attempts": 1,
    "next_attempt_at": "2026-09-23T08:00:00Z", "delivered_at": "2026-09-23T08:00:04Z",
    "abandoned_at": null, "last_status": null, "last_error": null}]}
```

`department_exists` is `false` for a destination naming a department nobody
has - one that will hear nothing until it exists.

**`POST /api/v1/webhooks/{name}/test`** - queues a `test` event for that one
destination, whatever it subscribes to, and answers `202` with its `event_id`.
It goes through the same queue and retries as any event, so the screen shows
how it went. `404` for a name that is not configured. Each test is recorded in
the [audit log](/kasl-server/reference/the-audit-log/) as `webhook.tested`.
