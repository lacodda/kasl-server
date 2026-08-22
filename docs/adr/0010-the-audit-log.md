# 0010. The audit log: a table, append-only, admin-only

Date: 2026-08-22

Status: Accepted

## Context

Roles arrived in 0.7.0 and departments in 0.8.0. An administrator can now issue
a token that writes someone's history, change who reports to whom, and reset a
password - and nothing beyond a log line records that any of it happened. Roles
without a trace do not earn trust, and the finance milestone (0.35.0) already
promises that "viewing salaries goes into the audit log", which needs somewhere
for it to go.

Two questions had to be settled before the table.

**What is recorded?** "Who looked at things and who changed things" are very
different in cost: changes are rare and bounded, reads are every request an
agent makes.

**Who reads it, and can it be cleared?**

## Decision

**Changes, plus reads that are explicitly marked sensitive.** Everything that
alters people, departments or agent tokens is recorded, along with sign-ins and
failed attempts. Ordinary reads are not.

Logging every request was rejected on arithmetic: ten agents uploading daily
produce hundreds of thousands of rows a year, all of them noise, and finding
the one entry that matters becomes the problem the log was meant to solve.
Logging only changes was rejected for a different reason - it leaves no place
for `salary.viewed` to be added later without inventing the concept of a
recorded read after the fact. The mechanism exists now; the first sensitive read
to use it arrives with the money.

Reading the audit log is itself not recorded. An audit of the audit buries the
actions under a log of people looking at the log.

**A table, not the log lines that already existed.** Tracing output is a stream:
answering "everything that happened to this person" means grepping across
rotated files that may not exist any more. Rows make it a `WHERE`, and put the
record wherever the database is backed up to.

**Administrators read it; nobody deletes from it.** There is no delete route at
all - not for old entries, not for a date range. A journal the watched party can
erase is not a journal, and the administrator is precisely the log's main
subject. Trimming it is an operation for whoever holds the database, which is a
deliberate speed bump rather than a button in a UI.

A manager is not admitted. The log records who changed what, and a manager can
change nothing (ADR 0008), so their view would consist entirely of other
people's actions.

**Nothing secret goes in.** An issued token is recorded as having been issued -
never its value. A password change is recorded as having happened, never what it
became, not even its length. A failed sign-in keeps the address that was tried,
because a run of failures against one account is the thing worth seeing, but not
the password that was typed - which is frequently a real password belonging to
another system.

**Writing an entry never fails a request.** The action being recorded has
already happened by the time it is logged. Refusing a token revocation because
its audit entry could not be written would undo nothing and turn a full disk
into an outage, so a failure is reported at error level and swallowed. Error
level on purpose: an audit log that quietly stops recording is worse than one
that never existed, because it is trusted.

## Consequences

- `actor_id` is `ON DELETE SET NULL` and the actor's email is kept as text, so
  an entry survives the account that made it and still says who it was.
- The action is text, not an enum: a new action must not need a migration, and
  the finance milestone adds its own.
- The table grows without bound and there is no trimming. A read is capped at
  500 entries so that no single request can ask for all of it.
- `ip` exists in the schema and is not yet filled: behind a reverse proxy it is
  whatever the proxy forwards, which is a decision about deployment rather than
  about this table.
- Entries are written after the fact and outside the transaction that made the
  change. A crash between the two leaves a change with no entry - rare, and the
  alternative is failing changes when the log is unavailable.
