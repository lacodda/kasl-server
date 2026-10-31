# 0027. Pay terms and their history

Date: 2026-10-31

Status: Accepted

## Context

The finance milestone turns the hours the agents send into money earned. Its
position was set when the milestone was planned: the money comes from the
honest hours of the tracker, not from a timesheet taken on trust, and the
module is not bookkeeping - no taxes, no holiday pay at average earnings, no
sick pay. It answers "what did these hours earn under these terms", not "what
must be paid under the law".

Two of the three inputs exist already: the hours worked on each date
(`workday_figures`, ADR 0023) and the norm each date owed (ADR 0017). The third
is how a person is paid, and it is the one that changes underneath everything
computed from it. A person is hired at one rate, raised in March, moved to a
salary in September. A raise must not reach back into February's money; a
mistake typed into March's rate must be correctable without leaving February
in doubt; and a later reader - an employee asking why August came to what it
did, or the period close of a later version - must be able to see which terms
applied to which date and when each was entered.

Four questions stood before the table.

**One record of terms per person, or a history.** A column on the user, as
`users.work_rate` is, answers "what now" and loses "what then": every
change rewrites the past it was meant to leave alone.

**A version with a start and an end, or with a start alone.** Ranges are the
obvious shape, and they can overlap or leave a gap - two rates on one date, or
none on a date the person worked.

**How a mistake is undone.** Editing the row is the obvious way, and it erases
the terms that money already computed may have used.

**How money is written down.** A rate is multiplied by hours, scaled by
multipliers, prorated by a norm. A float gets 0.1 wrong.

## Decision

**A history of versions, appended to and never rewritten.** `pay_terms` holds
one row per version: the person, the first date it applies to
(`effective_from`), the basis, the rate, the currency, an optional note, who
recorded it and when. Changing a rate is recording a new version. A version is
never updated, and never deleted except with the account it belongs to - which
in a running installation does not happen (accounts are deactivated, ADR 0007)
and in the demo happens on a reseed. A trigger in the database refuses both
an `UPDATE` and a direct `DELETE`, so the rule does not depend on every future
handler remembering it.

**A start and no end.** A version ends where the next one begins. Two versions
cannot overlap and cannot leave a gap between them, because there is nothing
to overlap with: the end of each is a fact derived from the next, not a second
fact stored beside it.

**The terms in force on a date are one rule.** Of the versions that started on
or before the date, the one with the latest start; of two with the same start,
the one recorded later. "Recorded later" is the larger `id`, a counter, rather
than a timestamp: two writes can share a clock reading, and a tie would leave
the date with two rates. The rule is a pure function (`pay::terms::in_force`),
and every accrual from the next version on asks it.

**A mistake is corrected by a later version with the same start.** It takes
the date over; the one it corrects stays in the history as `superseded`. The
history then says both what applied and what was entered by mistake and when it
was put right - which is what a person disputing a month needs to read.

**Back-dating is allowed while nothing is closed.** The first setup of an
installation is exactly that: a rate from the date of hire, over history
imported from kasl. "Past money is not recomputed when a rate changes" holds by
construction - a change from a date leaves every earlier date alone - and a
version deliberately starting in the past is a correction of the past, which
is legitimate until the period is closed. The close arrives later in this
milestone, and with it a refusal of any version starting inside a closed
period.

**Three bases, and the rest are layers.** `hourly` (hours worked × rate),
`daily` (days worked × rate) and `monthly` (a salary prorated by the norm).
Piecework and one-off bonuses are added on top of a basis rather than being
bases of their own, so "mixed" pay is a basis with layers, not a fourth enum
value that would have to say which mix.

**Money is decimal everywhere.** `numeric(18,4)` in the database,
`rust_decimal` in the server, and a string in JSON (`"25.50"`): a JSON number
is a float in every browser that reads it. A rate carries at most four decimal
places; a fifth is refused with `400` rather than rounded away by the column
without a word. The currency is an ISO 4217 code, checked against the
standard's list (the `iso_currency` crate, which also knows each currency's
minor unit - the rounding of accruals needs it). It belongs to the version: a
person moved to another currency gets a new version.

**Who reads it, until the finance privilege exists.** Administrators record and
read anybody's terms; each person reads their own. A department's manager does
not: they see the days, and money is the most sensitive thing this server will
hold. A separate privilege for it is a later version of this milestone, and it
arrives before any screen that shows money.

**The audit log records the fact, not the amount.** `pay_terms.recorded` names
the version, its start, basis and currency; `pay_terms.viewed` records an
administrator reading somebody else's terms. Neither carries a rate. The log is
read by administrators, and the finance privilege may separate the two: a rate
in the log would be read around it.

## Consequences

- Accruals have one place to ask what applied on a date, and the question has
  exactly one answer for every date.
- A history only grows. Its size is versions per person - a handful a year -
  and reading a person's terms is reading all of them.
- Correcting a version leaves it visible. That is the intent, and it means an
  administrator cannot make a typo disappear; they can only make it not apply.
- `users.work_rate`, the share of a full day, is still a single column, so a
  part-timer moved to full time back-dated moves the norm of the past - and,
  once salaries are prorated by it, the money. It needs the same treatment
  before salaries are computed.
- Units outside ISO 4217 (a stablecoin, an internal token) are refused. Whether
  an installation may define its own currencies is a question for the version
  that brings exchange rates.
