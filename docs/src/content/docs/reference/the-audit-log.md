---
title: The audit log
description: Every change to people, departments or agent tokens, plus sign-ins and the attempts that failed.
---

Everything that changes people, departments or agent tokens is recorded, along
with sign-ins and the attempts that failed:

```console
$ curl -H "Cookie: kasl_session=..." "http://127.0.0.1:8080/api/v1/audit?limit=2"
[{"id":4,"actor_id":null,"actor_email":"ivan@example.com","action":"auth.login_failed",
  "target_id":null,"target_label":null,"details":null,"at":"2026-08-22T00:58:12.880538Z"},
 {"id":3,"actor_id":"85440341-...","actor_email":"boss@example.com","action":"agent.issued",
  "target_id":"dcb60120-...","target_label":"ivan-laptop",
  "details":{"user_id":"9dce4dd0-..."},"at":"2026-08-22T00:58:11.850690Z"}]
```

Filter with `actor_id`, `target_id`, `action`, `since`, `until`, and page with
`limit` (500 at most) and `offset`. "Everything that happened to this person" is
`?target_id=...`.

**Nothing secret goes in.** An issued token is recorded as having been issued,
never as a value; a password change is recorded as having happened. A failed
sign-in keeps the address that was tried — a run of them against one account is
the thing worth seeing — but never the password, which is often a real one
belonging to somewhere else.

**There is no route to delete from it.** Not for old entries, not for a date
range. A journal the watched party can erase is not a journal, and the
administrator is the log's main subject; trimming it is an operation for
whoever holds the database. Reading the log is not itself recorded — an audit of
the audit buries the actions under a log of people looking at the log.

Only an administrator may read it. The reasoning is in
[ADR 0010](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0010-the-audit-log.md).
