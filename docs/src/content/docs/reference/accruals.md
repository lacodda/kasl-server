---
title: Accruals
description: GET /api/v1/me/accruals and GET /api/v1/users/{id}/accruals - what the hours earned under the terms in force, date by date, with overtime.
---

An accrual is what a person's hours earned under the
[pay terms](/kasl-server/reference/pay-terms/) in force on each date: an hourly
rate times the hours, or a day rate times the share of the day, with overtime
on top. It is **worked out on every read and never stored** - from the days the
agents sent, the norm of each date and the version of the terms that applied to
it - so a day uploaded late, or a version back-dated, simply changes the
answer. Closing a pay period, later in the finance module, is where a period's
figures stop moving.

The module is not bookkeeping: no taxes, no holiday pay, no sick pay. A salary
is accrued from the next version on. The reasoning is in
[ADR 0028](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0028-accruals-and-overtime.md).

## Reading them

```console
$ curl -H "Cookie: kasl_session=..." \
    "http://127.0.0.1:8080/api/v1/me/accruals?from=2026-10-05&to=2026-10-11"
{"user_id":"9dce4dd0-...","from":"2026-10-05","to":"2026-10-11",
 "days":[
   {"date":"2026-10-05","kind":"work","rest_day":false,"worked_seconds":32400,"norm_seconds":28800,
    "status":"accrued","terms":{"id":3,"basis":"hourly","currency":"USD"},
    "lines":[{"kind":"hours","rule":null,"seconds":32400,"share":null,"rate":"20.00","multiplier":"1","amount":"180.00"}],
    "amount":"180.00"},
   ...
   {"date":"2026-10-09","kind":"work","rest_day":false,"worked_seconds":32400,"norm_seconds":28800,
    "status":"accrued","terms":{"id":3,"basis":"hourly","currency":"USD"},
    "lines":[{"kind":"hours","rule":null,"seconds":14400,"share":null,"rate":"20.00","multiplier":"1","amount":"80.00"},
             {"kind":"overtime","rule":"weekly","seconds":18000,"share":null,"rate":"20.00","multiplier":"1.5","amount":"150.00"}],
    "amount":"230.00"}],
 "totals":[{"currency":"USD","amount":"950.00","worked_seconds":162000,"overtime_seconds":18000}],
 "not_accrued":{"no_terms_seconds":0,"salaried_seconds":0,"open_days":0}}
```

`from` and `to` are dates, both included, at most 400 days apart. Every stored
day in the range is listed, oldest first.

**A date earns the same whichever range asks.** Friday's weekly overtime
depends on Monday to Thursday, so the server reads each range out to whole
Monday-to-Sunday weeks and answers only the dates asked for: Friday alone comes
to exactly what it comes to inside its week.

## A day

| Field | What it is |
| --- | --- |
| `worked_seconds` | the day's figure, as every other screen reads it: start to end, less what was paused; `null` while the day is open |
| `norm_seconds` | what the date owed: zero on a rest day and on a day of leave |
| `rest_day` | the calendar asks nothing of the date - a weekend it did not move into the week, or a holiday |
| `terms` | the version in force on the date: its id, basis and currency |
| `status` | see below |
| `lines` | how the amount was made |
| `amount` | the sum of the lines, as a decimal **string** in the terms' currency; `null` unless accrued |

| `status` | Meaning |
| --- | --- |
| `accrued` | priced - the lines say how |
| `open` | the day has not ended, so it has no total to price yet |
| `away` | leave or illness with nothing worked; the tracker accrues no holiday or sick pay |
| `no_terms` | worked, but no terms were in force - which is not a rate of zero |
| `salaried` | paid by a salary, prorated over its month rather than a date at a time; accrued from the next version on |

## A line

Every line is **units times a rate times a multiplier**, all three written on
it, and its `amount` is that product rounded once to the currency's minor unit,
half a cent and up going up. A date is the sum of its lines, so the dates of a
month add up to the month to the cent.

| `kind` | Units | Rate |
| --- | --- | --- |
| `hours` | `seconds` of ordinary time | the hourly rate |
| `day` | `share` of the day, to four places | the day rate |
| `overtime` | `seconds` past a threshold, at the whole `multiplier` | the hourly rate, or the day's hourly equivalent |
| `overtime_premium` | `seconds` past a threshold that a day rate already pays for, at what the multiplier adds (`0.5` for 1.5) | the day's hourly equivalent |

`rule` says which threshold made an hour overtime: `daily`, `weekly` or
`rest_day`.

## How a date is priced

**By the hour:** every hour at the rate, and the hours past a threshold at the
rate times their multiplier.

**By the day:** the share of the day worked, at most the whole day. The day is
the date's norm; on a rest day or a day of leave, which owe nothing, it is the
person's own full day. Twenty minutes of an eight-hour day is `0.0417` of it.
Hours past an overtime threshold are paid on top at the day's hourly equivalent
- the rate over the day's hours, to four places - times the multiplier; an
overtime hour inside the hours the day already pays for earns only what the
multiplier adds. Ordinary time past the day earns nothing more: that is what a
day rate is.

**Overtime** is set with the terms ([below](#overtime-rules)). Where several
rules reach one hour - the tenth hour of a Saturday - it pays the largest
multiplier, never their product. An hour a daily step or a rest day already paid
more for does not count towards the weekly threshold; every ordinary hour does,
whatever terms it fell under. A raise in the middle of a week prices each date
by its own version and counts the week as one.

## Overtime rules

Overtime is part of a version of the terms, under `overtime` - see
[pay terms](/kasl-server/reference/pay-terms/#overtime). Three rules, each
optional:

```json
"overtime": {
  "daily":  { "after": "norm", "multiplier": "1.5", "then": { "after": "2", "multiplier": "2" } },
  "weekly": { "after": "40", "multiplier": "1.5" },
  "rest_days": "2"
}
```

| Law | Rules |
| --- | --- |
| United States (FLSA) | `weekly`: after `"40"`, `"1.5"` |
| California | `daily`: after `"8"`, `"1.5"`, then after `"4"`, `"2"`; `weekly`: after `"40"`, `"1.5"` |
| Russia (Labour Code arts. 152, 153) | `daily`: after `"norm"`, `"1.5"`, then after `"2"`, `"2"`; `rest_days`: `"2"` |
| Japan | `daily`: after `"8"`, `"1.25"`; `weekly`: after `"40"`, `"1.25"`; `rest_days`: `"1.35"` |

These are starting points to check against the law and the contract, not legal
advice.

## Who reads them

| Who | What |
| --- | --- |
| the person | their own, at `GET /api/v1/me/accruals` |
| administrators | anybody's, at `GET /api/v1/users/{id}/accruals` |
| the manager of a department | nothing - they see the days, not the money (`403`) |
| a [read token](/kasl-server/reference/the-bi-api/) | nothing |

An administrator reading somebody else's accruals is recorded in
[the audit log](/kasl-server/reference/the-audit-log/) as `accruals.viewed`,
with the range and never an amount.

## What it does not do yet

- **Salaries** (`monthly`) are answered as `salaried` and accrued from the next
  version on.
- **The threshold at the norm uses today's share of a full day** for every date:
  a person's share has no history yet, so a part-timer moved to full time
  back-dated moves the threshold of the past too.
- **Amounts in different currencies are never added**: `totals` has one entry
  per currency until exchange rates exist.
