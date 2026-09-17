---
title: Importing history from before the server
description: Bring an employee's own kasl SQLite history into the server without losing it.
---

Someone can track their time with kasl for a year before their team runs a
server. That history is an ordinary SQLite file on their machine, and it does
not have to be lost because the server arrived second:

```console
$ kasl-server import --db kasl.db --user employee@example.com --timezone -03:00
read 240 workdays, 312 pauses, 460 tasks from kasl.db
skipped 17 tasks the employee had deleted
imported 240 days as employee@example.com at -03:00
```

`--timezone` is required and has no default. kasl stores bare wall-clock text,
so nothing in the file says which offset it was recorded in - and a wrong guess
produces a perfectly plausible-looking year of work at the wrong hour. The
answer comes from whoever knows, and is echoed back so it is on the record.

- `--dry-run` reads and reports without writing anything.
- `--since` / `--until` bound the import by date, both ends inclusive. This is
  how someone who moved between time zones is imported correctly: one run per
  stretch, each with the offset that stretch was recorded in.
- The account must already exist - an import will not create it, so a typo in
  the email address cannot quietly file a year of history under a stranger.
- Re-importing replaces rather than duplicates, so a run that failed partway can
  simply be repeated, and a wrong offset is fixed by importing again with the
  right one.

The agent's file is opened read-only and never written to. Details and the
trade-offs behind the fixed offset are in
[ADR 0006](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0006-importing-local-history.md).
