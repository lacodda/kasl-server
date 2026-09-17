---
title: The data model
description: The tables migrations create, and the two deliberate differences from kasl's own schema.
---

Migrations live in `migrations/` and are applied on startup. The shape follows
kasl's own model, so a reader who knows the agent recognizes it:

| Table | Holds |
| --- | --- |
| `users` | People and their role: admin, manager, employee |
| `sessions` | Browser sign-ins; a token hash each, never the token |
| `departments` | Groups of people, each naming the manager who runs it |
| `audit_log` | Who did what, to whom, and when; append-only |
| `settings` | One row: how much detail this installation stores |
| `agents` | Installed kasl instances; a token hash each, never the token |
| `workdays` | One row per person per date: when the day started and ended, and - under a coarse privacy level - how much of it was paused |
| `pauses` | Idle stretches and manual breaks inside a day |
| `tasks`, `tags`, `task_tags` | What was worked on, and how it is labelled |
| `reports` | That a report was submitted, when, and with which figures |

Two differences from the agent's database are deliberate: instants are stored
with a time zone (the agent stores bare wall-clock text, which does not survive
a team spread across zones), and rows are tied together by foreign keys rather
than by comparing dates. Both are recorded in [ADR 0003](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0003-time-and-identity-in-the-schema.md).
