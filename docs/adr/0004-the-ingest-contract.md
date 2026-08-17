# 0004. The ingest contract: whole days, last upload wins

Date: 2026-08-14
Status: Accepted

## Context

kasl agents run on employees' machines and update on their own schedule. The
endpoint they call is the one part of this server that cannot be changed by
editing both sides at once: a working agent must keep working. Three questions
had to be answered before writing it.

**What is the unit of an upload?** The agent holds a local database of days,
pauses and tasks. It could stream individual rows, or send a day at a time.

**What happens when the same day arrives twice?** It will: the network drops
mid-request and the agent cannot know whether the write landed; and the
employee edits a finished day in kasl - fixes a task, adds a break they forgot
to record - which has to reach the server somehow.

**How does an agent prove who it is,** on a server that does not yet have
accounts, roles, or an admin UI to issue credentials from?

## Decision

**A day is the unit.** `POST /api/v1/days` takes one workday with its pauses
and tasks, written in a single transaction. Rows of a day are meaningless
apart: a day whose pauses were stored but whose tasks were not looks complete
to every reader, and nothing marks it for re-sending.

**The agent is the source of truth; the last upload wins.** A re-upload
replaces the stored day. The alternative - keeping the first version - would
mean an employee correcting a day in kasl while the server holds the wrong one,
with nothing to fix it until an editing UI exists. Idempotency follows from
this rather than being bolted on: the same payload sent twice leaves the same
rows, so a retry is safe by construction. Retries, backoff and backfill of an
offline stretch are the next milestone; this one settles the rule they rely on.

Within a day the two collections differ:

- **Pauses are replaced.** The agent splits and merges them as activity comes
  in, so they have no stable identity to match on. The set that arrives is the
  set that is stored, and a pause the employee deleted disappears here too.
- **Tasks are matched** on `(user_id, agent_task_id)`, the agent's own row id.
  The same task is often carried across several days; wiping a date's tasks
  would take yesterday's copy of it with them.

**Instants carry an offset; the day carries its own date.** Rejecting a
timestamp without an offset is deliberate, and it is the one requirement this
server places on kasl - which today stores bare wall-clock text. See ADR 0003.

**Agents authenticate with a bearer token, hashed at rest.** Until the admin UI
issues tokens, the operator declares them in `KASL_AGENTS` as `email:token`
pairs, applied idempotently on startup. There is no public enrollment endpoint:
adding one would give the server a door that opens without a secret, before it
has roles or an audit trail to notice who walked through. When token issuing
moves into the UI, the ingest contract does not change - only where the token
came from.

## Consequences

- kasl must send an offset and the local date. An agent that sends neither is
  refused with a 400 rather than silently misinterpreted.
- A misbehaving agent can overwrite its own history, and only its own. The
  token identifies the user; nothing in the payload chooses whose day it is.
- Deleting a pause on the agent deletes it here; deleting a *task* needs the
  explicit signal added in 0.4.0 (`tasks_are_complete`), since tasks are matched
  rather than replaced. See ADR 0005.
- Uploads are not authenticated per request beyond the token: replay of a
  captured payload writes what the agent itself would have written. Given the
  last-upload-wins rule, that is a re-run of a legitimate write, not a way to
  forge one.
