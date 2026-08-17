# 0005. Deletions and backfill: an authoritative day, a batch of them

Date: 2026-08-17
Status: Accepted

## Context

ADR 0004 settled how one day is uploaded and left two gaps that a real agent
runs into within a week of use.

**A deleted task never dies.** Pauses are replaced as a set, so removing one on
the agent removes it here. Tasks are matched on `agent_task_id`, because the
same piece of work is carried across several days and wiping a date's tasks
would take yesterday's copy with it. The consequence is that an employee who
deletes a task in kasl - a duplicate, something entered by mistake - leaves it
on their manager's dashboard forever, with nothing short of hand-editing the
database to remove it.

**A week offline is seven requests.** An agent that could not reach the server
holds a stretch of days. Sending them one at a time works, but every day is its
own round trip, its own chance to be cut off, and the agent learns nothing about
the run as a whole - only about the request in front of it.

## Decision

**A day may declare its task list authoritative.** `tasks_are_complete: true`
in the payload means "this is everything I hold for this date"; the server then
deletes the tasks it has *on that date* which the payload does not contain. The
deletion is scoped to the date, never to the user: a task that also appears on
another day keeps its row there, and an agent backfilling Monday cannot erase
Friday.

The flag defaults to false, and that default is the whole reason it is a flag.
An agent shipped before this existed sends its tasks and says nothing; reading
that silence as "delete the rest" would destroy an employee's history on the
first upload after a server upgrade. Deleting data is opt-in, stated, and made
by the side that knows.

The alternative was a tombstone list - the agent remembering `deleted_task_ids`
and reporting them. It is more precise in principle and worse in practice: it
needs a new table in kasl, a rule for when a tombstone may be forgotten, and it
still fails for a task deleted while the agent's database was restored from a
backup. The authoritative set needs the agent to remember nothing at all: it
sends the truth about a day, and the truth includes what is no longer there.

**Backfill is a batch of days, each its own transaction.** `POST
/api/v1/days/batch` takes an array and answers per day: which were accepted,
which were refused, and why. A day's write stays atomic, but one bad day does
not sink the others.

All-or-nothing across the batch was rejected for a specific failure it creates:
an agent holding one unacceptable day - a task the schema refuses, a corrupted
row - would be unable to deliver *any* of its backlog, and would retry the same
doomed request forever. Per-day results let it store what it can, report what it
cannot, and stop asking.

**The batch is bounded.** A day count limit (`KASL_MAX_BATCH_DAYS`) and a body
size limit are enforced and answered with `413`, so backfilling a year cannot
be indistinguishable from an attack on the server.

## Consequences

- kasl gains one boolean to set and one endpoint to prefer when it has a
  backlog. Neither is required: an agent that only ever posts single days
  without the flag keeps working exactly as it did.
- Retry and backoff stay on the agent, which is the only side that knows what it
  has not yet delivered. What this server owes is an honest signal about which
  failures are worth repeating: `4xx` means the payload will never be accepted
  as sent, `5xx` and `429` mean try again. That distinction is now documented
  rather than implied.
- A batch answers `200` even when some days were refused. The status describes
  the request, which was processed; the body describes each day. A client that
  checks only the status will not notice a rejected day, so the response names
  the counts first.
