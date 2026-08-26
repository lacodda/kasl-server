# 0011. The privacy manifest: a level the server enforces, and a page that says so

Date: 2026-08-26

Status: Accepted

## Context

kasl-server is self-hosted, and the person whose day it records is not the
person who installs it. That asymmetry is the whole adoption problem: an
employee is asked to run an agent that watches when they stop typing, and the
only answer available to them so far has been "read the source".

By 0.9.0 the server holds, per person and per day: when the day started and
ended, every pause down to the second with a free-text reason, every task by
name with its comment, and a productivity percentage. That is a great deal more
than "hours worked", and none of it was ever announced.

Two questions had to be settled.

**Is detail a policy, or is it fixed?** A promise the server cannot break is
worth more than a paragraph in a README, but enforcement means the installation
has to choose a level - and a wrong default either breaks the product or
breaks the promise.

**Who decides, and who may read the decision?**

## Decision

**A policy with three levels, enforced at ingest, defaulting to `full`.**

* `full` - everything the agent sends, which is what 0.1.0 through 0.9.0 did.
* `moderate` - pauses keep their times, but not the reason the employee typed;
  tasks keep their names, but not their comments.
* `coarse` - a day keeps its start, its end, and how much of it was paused, as
  a count and a total. Individual pauses are not stored, and neither are tasks.

The level is enforced on the way in, not on the way out. A field that a policy
excludes is dropped before the transaction that writes the day, so it never
exists in the database, never reaches a backup, and cannot be recovered by
changing the policy back. A filter applied at read time would be a promise
about the UI; this is a promise about the disk.

`full` is the default because the alternative is a breaking change disguised as
a virtue. Installations already running would silently start discarding data,
dashboards designed against detail (0.13.0) would be built on whatever the
timid default left, and the loss would be permanent and invisible. Narrowing is
the deliberate act, and it is one setting.

**The administrator sets it; everyone reads it.** `GET /api/v1/privacy` answers
for any signed-in person and for an authenticated agent - the agent case is the
point, because it lets kasl show the manifest in the CLI, where the employee
already is, rather than requiring them to log into the server that watches
them. `PUT /api/v1/privacy` is an administrator's, and the change goes into the
audit log (ADR 0010): a policy that can be quietly loosened is not a policy.

There is no per-employee opt-out. It was considered and rejected: a manager
comparing a team where one person's pauses are missing gets a dashboard that
lies by omission, and explaining the hole costs more trust than the setting
buys. The unit of the promise is the installation.

**The manifest states what is never stored, too.** Keystrokes, window titles,
application names, screenshots, URLs, and file paths appear in the response as
an explicit list of things the server has no column for. Absence is not
reassuring on its own - a reader cannot tell "we do not collect it" from "we
have not listed it".

**An upload says what it dropped.** The response to `POST /api/v1/days` carries
the level that applied and counts of what was discarded. An agent that sends
five pauses and is told "5 accepted" under a policy that stored none would be
reporting a delivery that did not happen, and the employee would believe their
break was recorded.

## Consequences

The ingest path gains a policy lookup. It is one small row read once per
upload, cached per request rather than per day, so a batch of thirty days pays
for it once.

Changing the level does not touch history. Tightening it stops new detail from
arriving and leaves what is already stored; loosening it does not bring back
what was dropped. This is stated in the manifest, because the alternative
reading - that tightening the policy erases the past - is the one an employee
would hope for and would be wrong about.

The manifest is generated from the level rather than written by hand, so it
cannot drift from what the server actually does. What it cannot check is that
the list of columns matches the schema; a test holds that instead.
