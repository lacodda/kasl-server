# 0028. Accruals and overtime

Date: 2026-11-10

Status: Accepted

## Context

ADR 0027 gave the finance module its terms: a history of versions, each with a
basis, a rate and a currency. This version turns them into money for the first
two bases - an hourly rate, and a day rate - together with overtime, whose
thresholds and multipliers touch both alike.

Five questions stood before the code.

**Stored or derived.** The norm (ADR 0017) and the signals (ADR 0016) are
derived on every read. Money is the thing most tempting to store: it is what a
person disputes, and a stored figure cannot drift. But until a period is closed
nothing about it is final - a day uploaded late, a version back-dated by an
administrator setting up an installation over imported history - and a stored
accrual would be a second copy of a computed fact that every one of those has to
remember to update.

**Where the overtime rules live.** "Overtime" is not one rule. The United
States pays half again past forty hours a week; California also past eight a
day, and double past twelve; Russia pays half again for the first two hours past
the day's norm and double after; Japan a quarter more past eight a day and forty
a week, a third more on a rest day. The shapes differ in whether the threshold
is fixed or is the norm, whether it counts the day or the week, and how many
steps it has.

**What a day rate pays.** Twenty minutes is not a day's work, and eleven hours
are not a day and three-eighths unless something says so.

**What happens where rules meet.** A Saturday that is also the tenth hour of a
day and the forty-fifth of a week falls under three rules at once.

**How money is rounded.** A rate times hours times a multiplier is rarely a
whole cent, and the parts of a month must add up to the month.

## Decision

**Accruals are derived on every read, never stored.** An accrual is a pure
function of the days (`workday_figures`, ADR 0023 - the figure every other
screen reads), the norm of each date, and the version in force on it
(`pay::terms::in_force`). The period close, later in this milestone, is where a
period's figures become a fact; until then a late day or a back-dated version
simply changes the answer, which is what an open period is.

**The rules are part of the version, as columns of its row.** A version carries
at most three rules, each optional: a daily ladder, a weekly ladder, and a
multiplier for rest days. A ladder is a threshold, a multiplier, and an
optional second step some hours further on that pays more. A threshold is
either fixed hours worked in the span or the span's own norm - the date's, as
the calendar and the person's share of a full day ask for it, or the week's
less any leave. Every law the module was planned against is one of these
shapes; none needs a third step.

Columns rather than rows of a table beside `pay_terms`: a version is history
that is never rewritten, and rules in rows of their own could be added to a
version after money had been computed from it. As columns they are exactly as
fixed as the rate, under the same trigger. And the database checks the shape -
a second step without its multiplier, a step that pays less than the one before
it, a fixed ladder that runs past the end of its day - so a wrong rule cannot
be stored by a writer that is not the API.

**A week is Monday to Sunday,** as on every screen of the server. Not a
setting: an installation whose law counts another seven days needs that rule,
not a knob that asks the operator which day their week starts.

**A day rate pays the share of the day worked, at most the whole day.** The day
is the date's norm; on a rest day or a day of leave, which owe nothing, it is
the person's own full day, and for a share of zero the installation's. Hours
past an overtime threshold are paid on top at the day's hourly equivalent (the
rate over the day's hours, to four places) times the multiplier - except that an
overtime hour inside the hours the day already pays for earns only what the
multiplier adds to it. Ordinary time past the day earns nothing more: that is
what a day rate is.

**No hour is paid twice for being over.** Where several rules reach one hour, it
pays the largest multiplier, never their product. An hour a daily step or a
rest day already paid more for does not count towards the weekly threshold;
every ordinary hour does, under whatever terms it was worked - including a day
with no terms in force. Each date uses the version in force on it, so a raise in
the middle of a week prices its two halves differently and counts them as one
week.

**A date earns the same whichever range asks.** The weekly threshold needs the
earlier days of the date's week, so a range is read out to whole weeks and cut
back afterwards. Each line is rounded on its own to the currency's minor unit
(`iso_currency`), half a cent and up going up, and a date is the sum of its
lines - so the dates of a month add up to the month and the months to the year,
to the cent.

**Every line can be checked by hand.** A line is units - hours, or a share of a
day to four places - times a rate times a multiplier, all three written on it,
and its amount is that product rounded once. Amounts in two currencies are never
added: the totals are one per currency until exchange rates exist.

**What is not accrued is said, not drawn as zero.** A day still open has no
total and earns nothing yet; a date with no terms in force is `no_terms`; a
date under a salary is `salaried` until the version that brings salaries; a day
of leave with nothing worked is `away`, because the tracker accrues no holiday
or sick pay.

**The same readers as the terms.** The person reads their own at
`/me/accruals`; an administrator anybody's at `/users/{id}/accruals`, recorded
as `accruals.viewed` with the range and no amount; a department's manager
nothing.

## Consequences

- One function prices every date, and the tests that hold it - a date alone
  against its week, a month against its days, every line against its own
  product - hold the handler too, through the router.
- Reading a range reads up to six days either side of it. Bounded, and the
  price of a weekly threshold that does not depend on how it was asked.
- The norm a threshold stands on uses today's share of a full day for every
  date, because `users.work_rate` has no history yet. A part-timer moved to
  full time back-dated moves the threshold of the past - the limit ADR 0027
  named, and the next version's to remove before salaries are prorated by it.
- Overtime of a salary - the hourly rate derived from a monthly one - is not
  here; it arrives with salaries, if it arrives.
- A rule beyond two steps, or a week that does not start on Monday, needs a
  migration and a new decision, not a value typed into a form.
