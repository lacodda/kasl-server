# 0020. Notifications to the employee

Date: 2026-09-25

Status: Accepted

## Context

Everything the server has said so far, it has said to a manager. Alerts wait
on the dashboard (ADR 0018) and go out to a team's chat (ADR 0019); the person
the alert is about hears nothing unless they sign in and go looking - and there
is nothing to look at, because no screen of theirs mentions it.

That is backwards for the one alert a person can do something about. A day the
server has held open for seventeen hours is, nine times out of ten, a close
kasl made on the laptop and never managed to send: the employee finished their
day, the manager sees one that never ended, and the only person able to fix it
is the only person not told. It is also backwards for the product's own
promise. The privacy manifest exists because the person being measured is not
the person who installed the server (ADR 0011); a server that tells a manager
"eleven hours on Tuesday" and says nothing to the person who worked them is
keeping a record *about* somebody rather than *with* them.

The plan named the events as "a report did not arrive" and "the week was
approved". Neither exists: the server receives no reports (the event joins when
the timesheet route does, v0.28), and nothing approves a week (v0.26). What
this milestone builds is the channel itself, with the events the server already
knows about - and each of those later milestones adds a kind to it rather than
a channel of its own.

## Decision

**A notification is a message to one person, and it is stored.** A table,
`notifications`, one row per thing said to one person. Stored for the reason an
alert is (ADR 0018): the moment it was said and whether it has been read are
not derivable from anything else, and a notification computed on read could
never be delivered to a machine that was asleep when it happened. It is written
in the transaction that made its fact true - the alert raised, the token
issued, the level changed - so the fact and the telling commit together.

**Four kinds, each about a fact the employee cannot see from where they are.**

* `alert.raised` - the server raised an alert about you. Your manager was just
  told; you are told the same sentence, with the same figures.
* `agent.issued` - a new token can now report as you. The same notice every
  account system sends for a new sign-in, and for the same reason: if it was
  not you, you are the only one who can tell.
* `agent.revoked` - a machine can no longer report as you.
* `privacy.changed` - what this server keeps about your days changed, from one
  level to another (ADR 0011). A manifest that changes silently is a manifest
  nobody can rely on.

Acknowledging an alert is not told. "Your manager decided it was fine" is a
comfort, not a fact to act on, and a channel that says everything teaches
people to ignore it.

**Nobody else reads them.** Not the manager, not an administrator. The facts
behind them are already visible to the people entitled to them - the alerts
feed, the audit log - and what the person was told is between the server and
them. `/me/notifications`, with no role and no department in the query, for the
same reason `/me/days` has none.

**Delivery to kasl is a pull on the pulse's cadence, with a cursor the server
keeps.** The server cannot reach an agent: laptops sit behind NAT and sleep.
Every pulse is already a round trip each minute (ADR 0014), so its answer
carries one more number - how many notifications this machine has not shown -
and an agent that sees a non-zero one reads `GET /api/v1/agent/notifications`
and then says `POST /agent/notifications/ack` up to the last one it showed.

The cursor lives on the server, one per agent, rather than in kasl. A
reinstalled kasl on the same token then does not replay a year of history, a
second machine gets its own toast rather than none (a notice missed on the
desk nobody is at is worse than the same notice twice), and the agent needs no
state for this at all. A machine issued a token today is not told what happened
before it existed.

Read and shown are different facts, kept apart. `ack` is "this machine
displayed it"; `read` is "the person has seen it" - the web inbox opened, or
kasl reporting a click. Something read is not toasted anywhere afterwards;
something toasted is still unread in the inbox until the person looks. Both are
cursors (per agent, per person), not flags on rows, because they only ever move
forward and a list read top to bottom is read up to a point.

**What is no longer true is not toasted.** A notification about an alert takes
its truth from the alert: once the alert has resolved - the day finally closed,
the agent came back - the notification is withdrawn, and the inbox shows it as
over rather than hiding it. Derived from `alerts.resolved_at` rather than
copied, so the two cannot disagree.

An agent is never told about its own silence. `no_agent_data` means nothing has
arrived from any of the person's machines; any machine able to ask has, by
asking, ended the condition, and the sweep that notices takes up to five
minutes. The notice stays in the web inbox, which is where a person away from
every machine could read it.

**The server writes the sentence, and the agent may do more with the fields.**
Each notification carries a `title` and a `body` in English, rendered by the
server from the stored facts, next to those facts in structured form. An agent
older than a kind shows the sentence and is right; an agent that knows the kind
can act on it - resend the day named in `alert.subject_date`, say. The same
arrangement the webhooks settled: a chat message is rendered from the payload,
so it can never say what the payload does not (ADR 0019). The web UI shows the
same sentences, so a person reading the toast and the inbox reads one text.

The figures are stored and never re-derived, for the reason ADR 0018 gives:
the sentence a person read is the one that was true when they were told.

## Consequences

The pulse answer grows a field, `notifications`. Additive, like every change to
a v1 response; an agent that does not know it ignores it.

`kasl`'s paired milestone is v3.3. Until then the agent routes have no client
in the wild and are exercised by this repository's tests - the same weaker
check ADR 0014 recorded for the pulse. The web inbox is used from this release.

The events a later milestone adds - a manager's note on a day (v0.25), a day
approved (v0.26), a report that did not arrive (v0.28), a pay period closed
(v0.42) - are a new value of `notification_kind` and a sentence, not a new
route.

A privacy change writes one row per active person. An installation of a
thousand people writes a thousand rows once, when an administrator changes one
setting; that is not a load worth designing around.

Alerts standing when notifications arrived are told once, by a migration
(0.24.1). A notice is written as its alert is raised, and an alert raised
before the table existed would otherwise never be told - the sweep does not
raise an open alert twice (ADR 0018). The first installation upgraded to
0.24.0 had exactly that: an open day, the case this milestone exists for, and
nobody told. Found by deploying rather than by a test, because every test
started from an empty schema; the suite now builds a database on the previous
migration, fills it, and upgrades it.

The table is in the backup and in the privacy manifest: "the notices this
server sent you, so you can read them again" is something stored about a
person, and a manifest that left it out would describe a quieter server than
the one running.
