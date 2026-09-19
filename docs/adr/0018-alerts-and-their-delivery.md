# 0018. Alerts, and why they are stored

Date: 2026-09-19

Status: Accepted

## Context

Every view this server draws waits to be opened. The signals were a real
advance on a table of totals - they say where to look rather than making the
manager notice - and they still say it on a page, in whole weeks, to whoever
happens to visit (ADR 0016).

The cases this milestone exists for are the ones where waiting is itself the
failure. An agent that died on Monday morning. A day left open across a
weekend, which will eventually produce a sixty-hour Monday nobody can
reconstruct. Somebody's eleventh hour on a Tuesday. None of those improve by
being found on Thursday, and two of them are worse by then: the open day has
quietly poisoned a week's total, and the dead agent has taken three days of
somebody's history with it.

ADR 0016 named this milestone in as many words and deferred to it: "there is
deliberately no alerting - nothing is sent anywhere. Delivery is its own
milestone, and a signal that arrives in a chat at 3 a.m. is a different
product decision from one that waits on a page."

## Decision

**An alert is a stored observation. A signal is not stored at all, and the two
are not inconsistent.**

This is the decision everything else here follows from, and the temptation to
be consistent with the signals - to compute alerts on read from the days, as a
fourth kind of signal - is the one that had to be refused. ADR 0016's reason
for keeping signals out of a table was exact: they are a function of the
workdays already in the database, so a `signals` table would be a second copy
of a derived fact, able to disagree with the days it came from.

An alert is not a derived fact. It carries two things that no workday can
produce:

* **When the condition began.** The absence of rows has no timestamp. A
  recomputation can say that a thing is true; only a row can say since when.
  "Quiet since Monday" is not in the data - the data is the silence.
* **What a person decided about it.** A manager who looked and concluded it
  was a holiday needs it to stop shouting. Nothing derived from the days can
  remember that, and an alert that cannot be answered trains people to ignore
  the whole column - which is the lesson `unknown` taught in ADR 0014, and the
  reason this product does not invent states.

So the rule is still computed from the days on every sweep, and what is
written down is **the event of having noticed**.

**The sweep reconciles; it does not insert.** Every five minutes it builds
what ought to be open right now and compares it with what is open. Three
outcomes per person and rule: the condition holds and nothing is open (raise),
it holds and something is open (leave it), nothing holds and something is open
(resolve). That is what makes a fortnight of silence one row instead of four
thousand, and what closes an alert when the agent comes back without anybody
clicking anything - which is how most of them will end.

**An alert that has fired never has its figures rewritten.** The row says "13
h of silence" for as long as it lives, even when the silence reaches 349 h.
Re-deriving the number on each sweep would make a sentence a manager already
read change underneath them, and the statement worth keeping is the one that
was true at the moment somebody was told.

**Three rules, and each is honest only now:**

* `no_agent_data` - nothing from any of this person's machines for longer than
  the installation's threshold. In hours and about *now*, which is what makes
  it a different object from the `no_data` signal: that one measures a
  person's own weekly rhythm and cannot speak before the week is complete.
  The two will sometimes both be true about the same person, and they are
  saying different things.
* `overwork` - a finished day ran past what that person owed it, by a share of
  their **own** norm. Before the production calendar this could only have been
  a number of hours invented here, which is this product asserting what a
  working day is on someone else's team - exactly what ADR 0016 refused. With
  a calendar and a rate it is arithmetic (ADR 0017), and it means the same
  thing for somebody on half time, where an installation-wide "over ten hours"
  would never mention a part-timer working double their day.
* `day_not_closed` - a day is still open long after any day plausibly runs.
  Judged by the wall clock and deliberately **not** by the norm: an open day
  has no total yet, so there is nothing to compare a norm with, and what is
  wrong with it is simply elapsed time.

**The thresholds are settings, and the rules are not.** This is the opposite
of what ADR 0016 decided for the signals, and the difference is who gets
interrupted. A signal is read by whoever opened the page, and a sensitivity
slider there is a knob on an opinion - a setting instead of a choice, which
that ADR correctly called a debt. An alert interrupts somebody, and how much
silence is worth interrupting over genuinely differs between a team in one
timezone and a team across four. What must not be configurable is the *set of
rules*: which things this server is willing to speak about is a choice, not an
operator's to invent.

**Answering an alert is not deleting it.** `acknowledged` and `resolved` are
separate states, because they are separate facts: one is "a person looked and
decided it needs no action", the other is "the condition stopped being true
and nobody had to do anything". Collapsing them would throw away the only
question worth asking of this table later - how much of what the server
shouted about was real.

An acknowledgement suppresses the rule for that person **for exactly as long
as the condition lasts.** Not forever: one click would then permanently mute a
rule for one employee, and nothing on any screen would ever say so. When a
sweep sees the condition go, the acknowledged row is stamped and stops
standing in the way; the condition returning later is a genuinely new event
and a new row.

**Delivery in this milestone is in-app.** A band at the top of the dashboard,
above the signals, because the thing that decays with time goes above the
thing that does not. Webhooks into a chat are the next milestone (v0.23) and
notifications back to the employee the one after (v0.24); the row written here
is precisely what those will ship outward. Delivery gets added to the record.
It does not replace it.

## Consequences

The sweep runs on a timer in the server process rather than on demand. That is
not an implementation detail: an alert computed when somebody opens the feed
has no `fired_at` worth the name - it would say the condition began the moment
the page was first opened - and it could never be delivered anywhere, which is
the whole of v0.23. Five minutes, because the conditions are measured in hours
and a finer interval would put a query over every account on a loop for
nothing anybody could act on.

A partial unique index on the open rows, not on all of them: one open alert
per person per rule, while a silence in March and a silence in July stay two
separate rows. This has a sharp edge that a test found rather than a reading -
an `ON CONFLICT` against a partial index does not see the acknowledged rows at
all, so the sweep has to skip those itself.

The silence rule reads **both** of an agent's stamps and takes the freshest:
`last_seen_at` (the token was used) and `heartbeat_received_at` (kasl is
watching somebody work). They answer different questions (ADR 0014), and an
agent can go a long time doing only the second - a machine whose employee is
on holiday pulses `idle` all week and uploads nothing at all. A first version
read only the first, and a live run against the demo flagged nine of twelve
people as silent, six of whom had pulsed seconds earlier. No unit test could
have caught it: every fixture set the two stamps together, and a fixture that
agrees with itself cannot produce the state where they disagree.

`overwork` looks at one day per person - the worst finished day in a
fortnight - rather than all of them. A manager does not need eleven rows to be
told about a week of eleven-hour days, and the next one surfaces once this is
answered.

A day whose norm is zero raises no overwork: a weekend, a holiday, a day of
leave. Any work at all on such a day exceeds its norm by an infinite share,
and "you worked on your holiday" is between the employee and their own screen,
not something this server raises with their manager.

Somebody with no live agent token raises nothing. An installation halfway
through handing tokens out would otherwise alert about every account on its
first afternoon, and the dashboard already says "no agents" in words - a more
useful sentence than "quiet for 400 hours".
