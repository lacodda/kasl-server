# 0003. Time zones, identity and ownership in the core schema

Date: 2026-08-14
Status: Accepted

## Context

The core schema stores what kasl agents record. The agent's own database is
single-user and single-machine, and its model reflects that:

- Instants are stored as wall-clock text (`%Y-%m-%d %H:%M:%S`) with **no UTC
  offset**. On one laptop that is unambiguous; a team spread over time zones
  makes it meaningless, and a daylight-saving shift makes it wrong.
- Rows are related by string comparison on the date: a pause belongs to a day
  because `date(pauses.start) = workdays.date`, with no foreign key.
- Primary keys are small sequential integers, unique only within that one
  installation.
- The agent decides which calendar date a piece of work belongs to, using the
  employee's local clock.

A server merging many agents cannot inherit those assumptions unchanged.

## Decision

**Instants are `timestamptz`; the employee's local date is stored alongside.**
Agents send an offset, and the absolute instant is what the server keeps.
`workdays.date` and `tasks.date` additionally carry the calendar date the agent
assigned, rather than deriving it from the instant — near midnight, after
travel, or in a server zone different from the employee's, a derived date is
not the employee's date, and "my Tuesday" is what every screen is about.

**Identity is `uuid`, generated server-side.** Agent-local integer ids collide
across employees by construction. The agent's ids are still stored, as
`tasks.agent_task_id` (the row) and `tasks.agent_group_id` (the agent's
`task_id`, tying the same work across days), because a re-upload has to find
the row it already sent — see ADR 0004 on idempotency when ingest lands.

**Ownership is expressed as foreign keys, with cascade deletes.** Every table
carries `user_id`, or reaches it through one (`pauses` → `workdays`,
`task_tags` → `tasks`). Deleting a user takes their data with it; deleting a
day takes its pauses. Relating rows by date comparison, as the agent does,
leaves orphans no reader can interpret.

**Uniqueness follows the agent where the agent is right.** One workday per
person per date, matching the agent's `UNIQUE(date)`; one task per
`(user_id, agent_task_id)`; one report per `(user_id, kind, period_start)`.
These are the constraints that turn a repeated upload into an update.

**A report is an event, not a copy of the day.** Hours and productivity are
recomputed from workdays and pauses whenever shown, exactly as the agent does —
kasl has no `reports` table at all. What cannot be recomputed is that a report
was submitted, when, and with which figures at that moment; approval and the
"missing or late report" signals rest on that, so those figures are frozen into
the row.

**Tags are per user.** Two employees both having "review" does not make it one
tag, and one person's vocabulary is not the team's.

## Consequences

- The ingest contract must include a UTC offset and the agent's local date.
  An agent that sends bare wall-clock time cannot be accepted correctly; this
  is a requirement on the kasl side of the pairing, not a detail of the server.
- Old agent data imported later (the history-import milestone) carries no
  offset. It has to be interpreted in a declared time zone at import time, and
  that choice belongs to whoever runs the import.
- `pauses.duration_seconds` is stored, not computed: the agent merges
  neighbouring pauses across a configurable gap, so the duration is not always
  `ended_at - started_at`.
- Cascade deletes make account deletion final. Employees who leave should be
  deactivated (`users.active = false`), which keeps their history; deletion is
  the tool for erasure requests.
