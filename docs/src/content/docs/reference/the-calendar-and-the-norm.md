---
title: The calendar and the norm
description: GET/PUT /api/v1/calendar - the production calendar, the installation's full day, and what each person owes.
---

The server answers hours. The calendar is what makes those hours mean
something: without it, "32 hours this week" is a number with no reading, and
with it the same number is either a week short of its norm or a four-day week
worked in full.

Three facts make a norm, and each is stored once
([ADR 0017](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0017-the-production-calendar-and-the-norm.md)):

| Fact | Where it lives | Default |
| --- | --- | --- |
| Which dates are unlike their weekday | `calendar_days` | empty |
| What a full day is here | `settings.standard_hours` | `8` |
| A person's share of a full day | `users.work_rate` | `1` |

## What the calendar holds

Only the exceptions. A year is about a dozen rows, which is also the size an
administrator can check against the decree those rows came from.

| Kind | What it means | Hours |
| --- | --- | --- |
| `holiday` | a working weekday that is not worked | none |
| `short_day` | the eve of a holiday | one hour less |
| `working_weekend` | a weekend day moved into the working week | a full day |

Weekends are not stored: Saturday and Sunday are not worked unless a
`working_weekend` row says otherwise. This is why a flat list of holidays
cannot express a production calendar — a list can only ever subtract, and a
calendar also adds.

No country is built in. The dates are an act of a particular government in a
particular year, revised by later acts; a dependency shipping last year's
answer would be confidently wrong and would still have to be checked by hand.

## Reading a year

```console
$ curl -H "Cookie: kasl_session=..." "http://127.0.0.1:8080/api/v1/calendar?year=2026"
{"year":2026,"standard_hours":8,
 "days":[{"date":"2026-01-01","kind":"holiday","note":"New Year"},
         {"date":"2026-11-07","kind":"working_weekend","note":"transferred"},
         {"date":"2026-12-31","kind":"short_day","note":null}]}
```

Anyone signed in may read it. Which days of the year are worked is not a secret
from the people working them, and an employee whose week asked for thirty-two
hours has to be able to see why.

## Writing a year

```console
$ curl -X PUT -H "Cookie: kasl_session=..." -H "Content-Type: application/json" \
    -d '{"days":[{"date":"2026-01-01","kind":"holiday","note":"New Year"}]}' \
    "http://127.0.0.1:8080/api/v1/calendar?year=2026"
```

Administrators only, one whole year at a time, and **a replacement rather than
a merge**: what you send is what the year holds afterwards. A corrected
calendar is the document that is right, and merging would leave last year's
wrong rows in place with nothing on any screen to say they are still there.

The write is scoped to the year in the query, so entering next year's calendar
never touches this one. A date outside that year is a `400` naming which entry
is wrong, and nothing is written; so is the same date twice.

## The installation's full day

```console
$ curl -X PUT -H "Cookie: kasl_session=..." -H "Content-Type: application/json" \
    -d '{"standard_hours":7.5}' "http://127.0.0.1:8080/api/v1/calendar/standard-hours"
```

Administrators only, and recorded in [the audit log](/kasl-server/reference/the-audit-log/)
as `calendar.standard_hours_changed`: this is the figure every norm on every
screen is computed from, so a change to it moves everybody's numbers at once.

Hours rather than a weekly total — the calendar already says which days are
worked, and a stored weekly figure would have to be reconciled with it on every
holiday.

## A person's share of it

```console
$ curl -X PUT -H "Cookie: kasl_session=..." -H "Content-Type: application/json" \
    -d '{"work_rate":0.5}' "http://127.0.0.1:8080/api/v1/users/{id}/work-rate"
```

A share, not a number of hours. Half time is `0.5`, and stays half time when
the installation moves to a seven-hour day and when a short day takes an hour
off the date. `0` is allowed for someone on the books who owes no hours; `2` is
the ceiling, which is a guard against `10` typed for ten hours a day rather
than a policy about overtime.

Its own route rather than a field on the user patch, and recorded as
`calendar.work_rate_changed`: it changes what every screen says about that
person, and an audit entry naming it is easier to find than one saying "user
updated".

## What the norm comes to

| The date is | Its norm |
| --- | --- |
| a weekend, with no `working_weekend` row | zero |
| a `holiday` | zero |
| a `short_day` | `(standard_hours - 1) x work_rate` |
| any other weekday, or a `working_weekend` | `standard_hours x work_rate` |

The rate multiplies last: half of a seven-hour eve is three and a half hours,
not a rounded seven halved.

**A day the employee was away owes nothing.** Agents send an optional `kind`
with each day — `work`, `vacation`, `sick`, `day_off` — and a day that is not
`work` is taken out of the norm rather than counted as worked. An agent that
sends no `kind` means `work`, which is what every kasl released before v1.35
says by saying nothing.

Nothing is ever stored as a computed norm. Calendars are corrected
retroactively; a derived norm simply becomes right when the row is fixed, while
a stored one would stay wrong in every row written before the correction.

## Where the norm shows up

`GET /api/v1/me/days` and `GET /api/v1/users/{id}/days` answer a `progress`
object beside the days, and each day carries its own `norm_seconds`:

```json
{"progress":{"norm_seconds":144000,"standard_hours":8,"work_rate":1},
 "worked_seconds":138600,
 "days":[{"date":"2026-09-14","kind":"work","norm_seconds":28800, "...":"..."},
         {"date":"2026-09-15","kind":"vacation","norm_seconds":0, "...":"..."}]}
```

`GET /api/v1/team/days` carries `norm_seconds`, `days_away` and `work_rate` on
every member, and `standard_hours` once for the table.

**A pair, never a percentage.** The server answers hours asked for next to
hours worked, and the screen divides them if it wants to: eight out of ten and
four out of five are the same percentage and not the same fact.

## What the norm is not

It is not a verdict. Nothing here calls a number good or bad and nothing ranks
people against each other — the [signals](/kasl-server/reference/where-to-look/)
still compare a person only with their own history. A norm makes "short of the
norm" sayable; the reasons a week is short are still things the server does not
know.
