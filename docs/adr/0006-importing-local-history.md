# 0006. Importing local history: a server subcommand, an offset the operator states

Date: 2026-08-17
Status: Accepted

## Context

kasl works alone. Someone can track their time with it for a year before their
team ever runs a server, and when the server does arrive that history is
sitting in an ordinary SQLite file on their machine. Throwing it away because
the server came second is the wrong answer: the whole promise of the product is
that the hours are honest, and a year of them is exactly what makes a first
dashboard worth looking at.

Two questions had to be settled before writing any of it.

**Where does the code that reads an agent's database live?** It could be a
subcommand of this server, a command in kasl that uploads its own history, or
an endpoint that accepts an uploaded file.

**Which time zone was the history recorded in?** kasl writes
`datetime(CURRENT_TIMESTAMP, 'localtime')` - bare wall-clock text, no offset
stored anywhere, in any row. The server's schema is built on absolute instants
(ADR 0003). Nothing in the file can bridge that gap.

## Decision

**The import is a subcommand of this server**: `kasl-server import --db <path>
--user <email> --timezone <offset>`. An operator copies the employee's file to
the server and runs one command.

The alternative - `kasl export` in the agent - is cleaner by layering, since
the server would then know nothing about SQLite. It was rejected on timing:
kasl has no sending code at all today (`ServerConfig` is an empty placeholder),
so that route puts the feature behind a kasl 3.x milestone and leaves this
version of the server with nothing. An upload endpoint was rejected too: it
would be the natural home for this eventually, but the server has no roles and
no admin login yet, and accepting someone else's database over an agent token
is a door that should not exist before there is anyone qualified to walk
through it.

**The offset is a required argument with no default.** `--timezone -03:00`, and
without it the import does not start. The value is echoed in the output,
because it is the one thing that cannot be checked afterwards by looking at the
data: every imported instant is stated relative to it, and a wrong offset
produces a perfectly plausible-looking year of work at the wrong hour.

A zone name (`America/Asuncion`) was considered and rejected for now. It would
handle daylight-saving transitions properly, which a year of history usually
crosses - a real advantage, honestly. But it costs a timezone database in the
binary, and it implies a precision the source data does not have: the agent's
rows carry no offset, so on the night the clocks go back there is no way to
tell which of the two possible 01:30s a row means. A fixed offset is a stated
approximation; a zone name would be an unstated one. If the approximation
proves painful, the argument can learn to accept both without changing anything
else - it is one parser.

**The agent's file is opened read-only.** It is the employee's only copy of
their own history, and the agent may still be running against it. This is not
observable from outside while the importer does not write - SQLite opens a
read-only file for writing without complaint and fails only at the first write
- so it stands as a guard against a future edit rather than something the tests
can demonstrate.

**Days are written one transaction each, and re-importing replaces.** The same
rule the ingest endpoint follows (ADR 0004), for the same reason: an import
that fails partway leaves what it managed, and re-running it - after fixing the
offset, say - corrects rather than doubles.

**An import will not create the account it imports into.** A typo in an email
address would otherwise file a year of someone's history under a person who
does not exist, with nothing to notice it. The user must exist first.

## Consequences

- The server depends on `rusqlite` and knows the agent's schema, which is a
  coupling that did not exist before. It is bounded: the reader touches four
  tables and tolerates the columns older agents lack (`tasks.deleted_at`, the
  whole `breaks` table).
- Data the server has no home for is dropped: `workdays.notes`, tags, task
  templates, the Jira inbox. Tags are the one worth revisiting - the schema has
  the tables (ADR 0003) and the import simply does not fill them yet.
- Imported history is indistinguishable from uploaded history once written.
  There is no "this came from an import" marker, so a wrong offset is fixed by
  re-importing with the right one, not by finding and correcting rows.
- Employees who tracked in more than one time zone over the imported period get
  one offset for all of it. Splitting the import by date range is the workaround
  and it needs no new code: two runs, two offsets, each covering its own file.
