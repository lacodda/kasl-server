# 0017. The production calendar and the norm

Date: 2026-09-17

Status: Accepted

## Context

Every screen this server draws so far answers with hours and stops there.
The heatmap shades a month without saying which square is a full day
(ADR 0015); the signals compare a person only with themselves, and say so out
loud, because there was no standard to compare them with (ADR 0016). Both
deferred the same thing to this milestone, by name.

The missing fact is small to state and easy to get wrong: **how much was this
person supposed to work on this date.** Without it "32 hours this week" is a
number with no reading. With it, it is either a week short of its norm or a
four-day week worked in full, and those are opposite facts.

Three things stand between the question and the answer, and each one is where
a naive version breaks.

**A calendar is not a list of holidays.** A production calendar transfers
days: a holiday falling on a Saturday moves to the following Monday, and the
Saturday before a long weekend becomes a working day. A list of dates that are
not worked can only subtract; it cannot say that a particular Saturday *is*
worked. It also cannot express the eve of a holiday, which in many countries
is one hour shorter - and an hour a day, unaccounted, is where a month's
timesheet stops balancing.

**A norm is not one number per installation.** Part-time exists in every team
of more than a few people. An installation-wide "40 hours a week" reports a
half-time employee as permanently half a person, which is precisely the reading
this product has refused everywhere else.

**A norm is not a number per person, either.** Store "20 hours a week" against
somebody and the short day before a holiday has to be subtracted from their
figure separately from everyone else's, and a change to the installation's full
day has to walk every account. Both are the kind of second copy that drifts
silently and is noticed at the end of a month.

## Decision

**The calendar stores exceptions, in three kinds.** `calendar_days` holds only
the dates that differ from the weekday they fall on: `holiday`, `short_day`,
`working_weekend`. A year is about a dozen rows, which is also the size an
administrator can check against the decree those rows came from. Weekends
themselves are not stored - Saturday and Sunday are not worked unless a
`working_weekend` row says otherwise.

No country is built in, and no calendar library is pulled in. The set of dates
is an act of a particular government in a particular year, revised by later
acts; a dependency shipping last year's answer would be confidently wrong, and
would still have to be checked by hand. The rows are administered through the
API, audited like every other administrative change, rather than read from an
environment variable an operator edits and restarts - which leaves nothing
behind saying who changed the year's calendar or when (the same argument that
put the privacy level in a table, ADR 0011).

**The norm is two facts multiplied.** `settings.standard_hours` is the
installation's full day, eight by default. `users.work_rate` is a person's
share of it, one by default. Half time is `0.5`, and stays half time when the
installation moves to a seven-hour day and when a short day takes an hour off
the date. Neither fact repeats the other, and neither has to be walked when the
other changes.

**The norm for a date is derived, never stored:**

| the date is | its norm |
| --- | --- |
| a weekend, without a `working_weekend` row | zero |
| a `holiday` | zero |
| a `short_day` | `(standard_hours - 1) x work_rate` |
| any other weekday, or a `working_weekend` | `standard_hours x work_rate` |

A stored norm would be a second copy of a derived fact - the same objection
that kept signals out of a table (ADR 0016). Calendars are corrected
retroactively, a decree is published late, somebody typed the wrong January;
a derived norm simply becomes right when the row is fixed, while a stored one
stays wrong in every row written before the correction.

**A day's own kind overrides the calendar.** `workdays.kind` - `work`,
`vacation`, `sick`, `day_off` - arrives with the day from the agent
(`kasl day off`, v1.35) and is optional: an agent that says nothing means
`work`, which is what every agent shipped before the field says by saying
nothing (ADR 0004). A day of leave or illness owes no hours. Without this the
dashboard would report a fortnight of holiday as eighty hours missing, which
is the most alarming possible way to be wrong about somebody on a beach.

**Progress is reported as a pair, not a percentage.** Every endpoint that
gains a norm answers `worked_seconds` next to `norm_seconds`, and the screen
does the division. A server that answered "80%" would have decided that eight
hours out of ten is the same fact as four out of five, and would have thrown
away the two numbers a person actually reads.

**The norm never becomes a verdict.** Nothing here calls a number good or bad,
nothing ranks people, and the words on the screen stay what they have been: a
figure and what it is measured against. A norm makes "short of the norm"
sayable; it does not make it a judgement, and the reasons a week is short are
still things the server does not know (ADR 0016).

## Consequences

The heatmap and the signals can finally use a threshold, and deliberately do
not yet: the shading rule and the three signals keep comparing a person with
themselves. Changing them is a separate decision with its own reasoning, and
bundling it into the milestone that merely makes it *possible* would bury it.

`unusual_week` stops firing on public holidays for installations that fill the
calendar in, because a week's norm falls with its holidays - which is the
defect ADR 0016 accepted in as many words, now closable.

An installation that enters no calendar gets the sensible default rather than
an error: weekdays are full days, weekends are not worked. This is wrong
around public holidays, and it is wrong in the direction of "this week looks
short" rather than a refusal to answer.

`work_rate` tops out at two. Not a policy about overtime - a guard against a
typo entering `10` for ten hours a day and quietly making one person's norm
the size of a department's.
