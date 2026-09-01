# 0016. Signals and the weekly trend

Date: 2026-09-01

Status: Accepted

## Context

The dashboard now shows a week of totals, a live status, and a month of
squares. All three are true, and all three still leave the manager to do the
noticing. Somebody whose hours have been sliding for three weeks looks
ordinary in any single view: this week's bar is a little shorter than last
week's, which is not a fact anyone acts on. The pattern only exists across
weeks, and nobody scrolls back through weeks to find it.

That is what this milestone answers: not more data, but *where to look*.

The temptation is to answer it with a norm - "under six hours is a problem",
"over ten is overwork". This server has no norms. They arrive with the
production calendar (v0.21), and any threshold invented before then would be
this product asserting what a working day should be, on installations whose
teams work part-time, four-day weeks, or across three continents. The same
argument kept the heatmap's shading on the client (ADR 0015).

## Decision

**A person is compared with themselves, never with a standard and never with
a colleague.** Every signal here is a statement about one person's own
history: their hours fell relative to their own weeks, their data stopped
relative to their own rhythm, this week is unlike their own usual. Nothing
compares Anna to Boris, and nothing calls a number good or bad.

This is a privacy position as much as a statistical one. A dashboard that
ranks people against each other turns a time tracker into a scoreboard, which
is the product this deliberately is not - and the manifest already promises
that what is stored is what the employee's own agent reported (ADR 0011).

**Three signals, and each one names what the server actually knows:**

* `declining` - the recent three weeks sit at least fifteen per cent below the
  three before them, comparing the median of each side. Three, not two: two
  weeks is one bad week next to one ordinary one, and a dashboard that flagged
  it would fire at everybody who took a Friday off.

  **Levels, not steps** - and this was the second answer, not the first. The
  first version asked for three weeks each lower than the last, which reads
  well and is wrong: a live run against the demo's deliberately fading person
  produced 33 → 24.8 → 27 → 20.9 → 23.1 → 22.1 h, an unmistakable slide that
  is never three falls in a row. One ordinary week resets a run, so the strict
  rule stayed silent on precisely the case this milestone exists for, while
  the medians see the same weeks as a nineteen per cent drop. Real decline is
  a change of level, not a monotonic sequence, and no amount of test data
  invented alongside the code would have said so - the tests all passed.
* `no_data` - nothing recorded for longer than this person's own usual gap.
  The employee whose agent died is the case the dashboard exists for, and the
  live status only sees it once a pulse is expected - an agent too old to
  send one is invisible there.
* `unusual_week` - the last complete week is far from that person's own median
  week, in either direction. Both directions on purpose: a week at half the
  usual hours and a week at twice them are equally worth a look, and flagging
  only the low one would make the signal an accusation rather than a question.

**A signal is a question, not a verdict.** Every one carries the figures it was
computed from, so the manager reads "8.5 h → 5.0 h over three weeks" rather
than a badge that says "problem". The words on the screen say what happened;
they never say what it means. Somebody's hours falling for three weeks is a
holiday, a hospital, a project that ended, or nothing at all, and the server
knows none of that.

**Weeks are complete weeks, and the current one never counts.** A Tuesday is
not a short week, but that is exactly what a partial week looks like to
arithmetic - so the current week is excluded from every calculation. Including
it would fire `declining` on the whole team every Monday morning.

**The median, not the mean.** One fourteen-hour crunch week drags a mean far
enough to hide a real decline behind it, and a single week off drags it the
other way. The median of a person's recent weeks is what "usual for them"
means to a person reading the screen, so it is what the code uses.

**The trend is twelve weeks, and it is drawn where the person is.** The
signals live on the dashboard, because that is where a manager already looks
and a signal on a page nobody opens is not a signal. The weekly chart lives in
the drill-down the signal links to: the number that made the server speak up
belongs next to the days that produced it.

## Consequences

Signals are computed on request rather than stored. They are a function of the
workdays already in the database, and a `signals` table would be a second copy
of a derived fact - one that could disagree with the days it came from, and
one somebody would eventually have to invalidate. The cost is a query over
twelve weeks of one team, which is the same order as the heatmap's month.

The fifteen per cent threshold comes from that same live run: it has to catch
a real nineteen per cent slide without firing on the ordinary wobble of weeks.

**No configuration.** Not "three weeks" as a setting, not a sensitivity slider.
A setting instead of a choice is a debt: it moves the decision onto the
operator, who has less information than we do, and every threshold then has to
be defended in two places. If three weeks proves wrong, it changes here.

There is deliberately no alerting - nothing is sent anywhere. Delivery is its
own milestone (v0.22), and a signal that arrives in a chat at 3 a.m. is a
different product decision from one that waits on a page.

`unusual_week` will fire on a legitimately short week - a holiday, a
conference - and that is accepted. Until the production calendar exists the
server cannot tell a holiday from an absence, and a signal that says "this
week was unlike their usual" is *true* in both cases. Naming the figures
rather than the conclusion is what keeps that honest.
