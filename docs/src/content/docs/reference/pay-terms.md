---
title: Pay terms
description: GET/POST /api/v1/users/{id}/pay-terms and GET /api/v1/me/pay-terms - how a person is paid, every version of it.
---

Pay terms say how a person is paid: by the hour, by the day or by the month,
at what rate, in which currency, from which date. They are the first part of
the finance module, whose position is set from the start: money earned is
computed from the hours the agents send, not from a timesheet taken on trust,
and the module is not bookkeeping - no taxes, no holiday pay, no sick pay.

Terms are a **history**, not a field. A raise is a new version; the old one
stays, and keeps governing the dates before the raise. The reasoning is in
[ADR 0027](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0027-pay-terms-and-their-history.md).

## A version

| Field | What it is |
| --- | --- |
| `effective_from` | the first date the version applies to |
| `basis` | `hourly` (per hour worked), `daily` (per day worked) or `monthly` (a salary) |
| `rate` | the amount per hour, day or month, as a decimal **string** |
| `currency` | an ISO 4217 code |
| `note` | why the terms changed, optional - read by the person too |
| `state` | where the version stands today - see below |
| `recorded_by`, `recorded_at` | who entered it, and when |

A version has a start and no end: it ends where the next one begins. Two
versions can therefore never overlap, and there is never a gap between them.
Before a person's first version they have no terms at all - which is not the
same as a rate of zero.

**The version in force on a date** is the one with the latest start on or
before it. Of two with the same start, the one recorded later.

| `state` | Meaning |
| --- | --- |
| `current` | in force today |
| `upcoming` | starts after today |
| `ended` | was in force until a version with a later start took over |
| `superseded` | corrected: a version with the same start was recorded after it, and this one never applies |

"Today" is the server's date, like every today on this server.

## Reading a person's terms

```console
$ curl -H "Cookie: kasl_session=..." http://127.0.0.1:8080/api/v1/users/9dce4dd0-.../pay-terms
{"user_id":"9dce4dd0-...","as_of":"2026-10-31",
 "versions":[
   {"id":3,"effective_from":"2026-03-01","basis":"hourly","rate":"25.00","currency":"USD",
    "note":"typo in the March raise","state":"current",
    "recorded_at":"2026-10-31T14:02:11.501Z","recorded_by":{"id":"85440341-...","name":"Boss"}},
   {"id":2,"effective_from":"2026-03-01","basis":"hourly","rate":"2500.00","currency":"USD",
    "note":null,"state":"superseded",
    "recorded_at":"2026-10-31T14:01:40.093Z","recorded_by":{"id":"85440341-...","name":"Boss"}},
   {"id":1,"effective_from":"2026-01-01","basis":"hourly","rate":"20.00","currency":"USD",
    "note":null,"state":"ended",
    "recorded_at":"2026-10-31T13:58:02.774Z","recorded_by":{"id":"85440341-...","name":"Boss"}}]}
```

Versions come latest start first, and of two with the same start, the later
recorded first. A person with no terms answers an empty list.

Each person reads their own at **`GET /api/v1/me/pay-terms`** - the same
document, every version and every note.

## Recording a version

```console
$ curl -X POST -H "Cookie: kasl_session=..." -H "Content-Type: application/json" \
    -d '{"effective_from":"2026-03-01","basis":"hourly","rate":"25.50","currency":"USD","note":"annual review"}' \
    http://127.0.0.1:8080/api/v1/users/9dce4dd0-.../pay-terms
```

Answers `201` with the person's whole history, because one version can change
the state of another: the one it follows becomes `ended`, the one it corrects
`superseded`.

- **The rate is a string.** `"25.50"`, never `25.5`: a JSON number is a float
  in every browser that reads it, and money is not. A number is refused with
  `422`. At most four decimal places - a fifth is refused with `400` rather
  than rounded. It must be more than zero.
- **The currency is ISO 4217**, in either case: `usd` is stored as `USD`. A
  retired code is refused with its successor named (`HRK has been replaced by
  EUR`), and so are codes that are not money anybody is paid in - gold, test
  codes, `XXX`. Units outside the standard, such as a stablecoin, are refused.
  A rate is written back with its currency's minor unit: `"25.50"` dollars,
  `"350000"` yen, `"0.125"` dinars.
- **Unknown fields are refused** with `422`, so a note sent as `notes` does not
  quietly go missing.
- **The start may be in the past.** Setting a person up means a rate from the
  date they were hired, over history imported from kasl. A version that starts
  in the past changes what those dates earned; closing a pay period, later in
  the finance module, will refuse a version starting inside a closed one.

## Changing and correcting

There is no `PUT` and no `DELETE`. A version is never rewritten and never
removed; the database refuses both, whatever a later handler might try.

- **A raise** is a version with a later start.
- **A mistake** is corrected by a version with the **same start**: it takes the
  date over, and the one it corrects stays in the history as `superseded`. The
  history then says both what applied and what was entered by mistake - which
  is what a person disputing a month needs to read.

## Who reads them

| Who | What |
| --- | --- |
| the person | their own, at `/me/pay-terms` |
| administrators | anybody's, and record them |
| the manager of a department | nothing - they see the days, not the money (`403`) |
| a [read token](/kasl-server/reference/the-bi-api/) | nothing |

An administrator reading somebody else's terms is recorded in
[the audit log](/kasl-server/reference/the-audit-log/) as `pay_terms.viewed`;
recording a version is `pay_terms.recorded`. Neither entry carries the rate.
The terms are listed in [what the server stores about
you](/kasl-server/concepts/what-the-server-stores-about-you/), with who reads
them.
