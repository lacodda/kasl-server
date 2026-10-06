---
title: Reports and approval
description: Reporting a finished day, and - where the installation asks for it - a manager approving it or sending it back.
---

A report is the person saying a day is finished, at the figures it came to -
what `kasl report --send` has always done at the end of a day, received by the
team server. Where the installation asks for it, a report is also a question to
the person's manager: approve these hours, or send them back with a reason.

## Reporting a day

From the web, for the person signed in:

```console
$ curl -X POST -H "Cookie: kasl_session=..." -H "Content-Type: application/json" \
    -d '{"date":"2026-10-05"}' http://127.0.0.1:8080/api/v1/me/reports
{"id":"5c0e...","user_id":"9dce4dd0-...","date":"2026-10-05","status":"submitted",
 "submitted_at":"2026-10-05T21:14:02Z","kind":"work",
 "started_at":"2026-10-05T12:02:00Z","ended_at":"2026-10-05T21:10:00Z","worked_seconds":27480,
 "reviewed_at":null,"reviewer_id":null,"reviewer":null,"reason":null}
```

From kasl, with the agent's token - the same act, written by the same code:

```console
$ curl -X POST -H "Authorization: Bearer $KASL_TOKEN" -H "Content-Type: application/json" \
    -d '{"date":"2026-10-05"}' http://127.0.0.1:8080/api/v1/agent/reports
```

The figures are the day's as the server holds it at that moment - kind, start,
end, and hours worked, computed the way [your days](/kasl-server/reference/reading-your-own-days/)
show them - and the report keeps them. `201` when the report is written.

**Sending it again is safe.** While the day's newest report still stands for the
figures the day comes to now, and nobody sent it back, the same request answers
that report with `200` and writes nothing. A retry after a lost connection does
not put a second copy in anybody's queue.

| Refused with | When |
| --- | --- |
| `404` | there is no day on that date - kasl has not sent it |
| `409` | the day is still open; finish it in kasl first - an open day has no total |

A report is accepted whether or not the installation approves days. Off, it is
the person saying the day is finished, and nothing more.

## Where a report stands

On the day it is about, in the answer to `/me/days` and `/users/{id}/days`
alike - the day's newest report, as `report`, beside a top-level `day_approval`
that says whether this installation approves days:

| `status` | Meaning |
| --- | --- |
| `submitted` | Reported, and nobody has answered. With approval on, it waits for a manager. |
| `approved` | A manager approved these figures, and the day still comes to them. |
| `returned` | A manager sent it back; `reason` says why. Report the day again. |
| `changed` | The day no longer comes to what was reported: kasl sent it again with other figures, or reopened it. Report the day again. |

**The status is never stored.** It is the report's figures compared with the day
as it is now, on every read. kasl stays the source of truth for the day - an
upload is never refused because a manager answered - so a day corrected after it
was approved is stored as sent, and its report reads `changed`: the approval was
of the figures in it, not of the date. A day sent again unchanged, as a retry or
a backfill does, still matches and stays approved.

A report is an event and is never edited. Reported again, a day has a second
report; the first stays, with whatever was said about it.

## Turning approval on

```console
$ curl -X PUT -H "Cookie: kasl_session=..." -H "Content-Type: application/json" \
    -d '{"enabled":true}' http://127.0.0.1:8080/api/v1/reports/approval
{"enabled":true}
```

An administrator's setting, off by default and recorded in the audit log.
`GET /api/v1/reports/approval` answers it to anyone signed in. Turning it off
answers nothing and erases nothing: approvals stand as given, and reports nobody
answered stop waiting until it is on again.

## What waits for a manager

```console
$ curl -H "Cookie: kasl_session=..." http://127.0.0.1:8080/api/v1/team/reports
{"day_approval":true,"waiting":2,
 "reports":[{"id":"5c0e...","date":"2026-10-05","status":"submitted","worked_seconds":27480, ...,
             "display_name":"Tomas Verhoeven","department":"Engineering"}, ...]}
```

The newest report of every day of everybody the reader may see - the manager's
department, or everyone for an administrator - that nobody has answered and that
still stands for its day, oldest day first, at most 200 with `waiting` counting
them all. Never the reader's own. A report whose day changed is not here: it is
waiting for its person, not for a manager.

## Approving

Any number at once - "approve the week" is one request:

```console
$ curl -X POST -H "Cookie: kasl_session=..." -H "Content-Type: application/json" \
    -d '{"ids":["5c0e...","71d2..."]}' http://127.0.0.1:8080/api/v1/reports/approve
{"approved":[{"id":"5c0e...","status":"approved","reviewer":"Priya Raman", ...}],
 "refused":[{"id":"71d2...","error":"the day changed after it was reported; it is approved once the day is reported again"}]}
```

Each report is decided on its own, and one that cannot be approved is listed
with the reason without stopping the rest - as a day in an upload batch is.
A report is refused when the reader cannot see it (`no such report`, the same as
one that does not exist), when it is their own, when a newer report of the same
day has arrived, when it was returned, or when its day changed. Approving a
report that already stands approved answers it as approved and changes nothing.
Up to 500 ids per request.

## Sending one back

```console
$ curl -X POST -H "Cookie: kasl_session=..." -H "Content-Type: application/json" \
    -d '{"reason":"Friday is missing its lunch break."}' \
    http://127.0.0.1:8080/api/v1/reports/5c0e.../return
```

The reason is required, and at most 1000 characters - the person has to know
what to look at. A report waiting for an answer can be returned, and so can one
already approved. One already returned cannot be returned again (`409`): the
next word is the person's, and they give it by reporting the day again -
corrected, or unchanged if they think it is right.

| Refused with | When |
| --- | --- |
| `400` | no reason, or one longer than 1000 characters |
| `403` | an employee; or the report is your own |
| `404` | a report you cannot see, or none at all |
| `409` | approval is off; the report was returned already; a newer report of the day arrived |

Both answers need approval to be on: with it off, `/reports/approve` and
`/reports/{id}/return` answer `409`.

## What the person is told

`report.approved` - once per approval, not once per day: "Priya Raman approved 3
of your days", listing them with the figures they were approved at.
`report.returned` - who and which day, and the reason is the sentence. See
[what you are told](/kasl-server/reference/what-you-are-told/).

## What else keeps a record

The [audit log](/kasl-server/reference/the-audit-log/) records
`report.approved` and `report.returned` against the person, with the report and
the date - never the reason - and `reports.approval_changed` for the setting.
The reason is kept once, on the report, where the person reads it. Reports are
in the backup and in [the privacy manifest](/kasl-server/concepts/what-the-server-stores-about-you/)
at every level. They are not sent to [webhooks](/kasl-server/reference/webhooks/).

The reasoning is in [ADR 0022](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0022-reports-and-approving-days.md).
