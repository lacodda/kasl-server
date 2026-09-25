---
title: What the server stores about you
description: The privacy manifest the server enforces and can recite - three levels, and what each keeps.
---

An employee is asked to run an agent that notices when they stop typing. The
honest answer to "what does it send" is one the server enforces and can recite,
so it is an endpoint rather than a paragraph:

```console
$ curl -H "Authorization: Bearer $KASL_TOKEN" http://127.0.0.1:8080/api/v1/privacy/agent
{"level":"full",
 "summary":"This server stores your working hours, every interruption with the reason you gave for it, and the tasks you logged with their comments.",
 "stored":[{"what":"workdays","detail":"the date, when the day started, when it ended, and whether you marked it as leave, sick or a day off"},
           {"what":"pauses","detail":"each interruption: when it began, how long it lasted, ..."},
           {"what":"tasks","detail":"what you logged: the name, your comment, and how complete you marked it"},
           {"what":"pause reasons","detail":"the text you type when you take a break by hand"},
           {"what":"account","detail":"your email, display name, role, department, ..."},
           {"what":"live status","detail":"whether your agent currently reports you as working, on a break, ..."},
           {"what":"notifications","detail":"what this server has told you - ... - and how far you have read; readable by you alone"}],
 "never_collected":["keystrokes or what you type","window titles","which applications you run",
                    "screenshots or camera images","web pages you visit","file names or paths","your location"],
 "visible_to":["you, in your own account","the manager of your department","administrators of this installation"],
 "sent_elsewhere":[{"to":"a Slack channel (team)","about":"everyone",
                    "what":["alerts about you as they are raised and cleared: your name, your department, and the figure behind each one - ...",
                            "the name of whoever answered an alert about you"]}],
 "retention":"Kept for as long as the installation keeps it: there is no automatic deletion. ...",
 "on_change":"Changing this setting affects what arrives from now on. ..."}
```

An agent reads it with its own token, so kasl can show it in the CLI - where the
employee already is - instead of requiring a login to the server that watches
them. Anyone signed in reads the same manifest at `GET /api/v1/privacy`.

**What leaves the server is listed too.** `sent_elsewhere` names every chat or
system the installation sends messages about people to - by the label the
operator gave it, never by its address - with whom it is about and what each
message carries. It is built from the same configuration the server sends
through, so it cannot list a channel that is not there or miss one that is.
Nothing configured, and the list is empty rather than absent: an empty list is
a promise, a missing one is not. See
[Sending alerts to a chat](/kasl-server/guides/sending-alerts-to-a-chat/).

## Three levels

How much detail an installation keeps is a setting, and the server enforces it
on the way in. A field a level excludes is dropped before the day is written: it
never reaches the database, never reaches a backup, and no later change of mind
can recover it.

| Level | Workdays | Pauses | Tasks | Free text |
| --- | --- | --- | --- | --- |
| `full` (default) | hours | each one, timed | name and comment | kept |
| `moderate` | hours | each one, timed | name only | dropped |
| `coarse` | hours | how many, how long in total | not stored | dropped |

The [live status](/kasl-server/reference/who-is-working-now/) sits outside this table on purpose. The
levels govern what is stored about a day; the pulse is the agent saying what is
happening right now, and the server keeps only the latest one - replaced each
time it arrives, never accumulated into a record of when you were at your desk.
It is named in the manifest at every level, because a manifest that listed only
days would describe a quieter server than the one running.

Under `coarse` a day still records how much of it was paused, as a count and a
total. Without that the day would claim uninterrupted work, which is a more
flattering picture than the truth and a false one.

`full` is the default because the alternative is a breaking change disguised as
a virtue: a timid default would silently start discarding data in installations
already running. Narrowing is the deliberate act, and it is one request:

```console
$ curl -X PUT -H "Cookie: kasl_session=..." -H "Content-Type: application/json"        -d '{"level":"moderate"}' http://127.0.0.1:8080/api/v1/privacy
```

Only an administrator may set it, and the change goes into the audit log with
both ends of it - a policy that can be quietly loosened is not a policy.

**An upload says what it dropped.** The response to `POST /api/v1/days` carries
the level that applied, and a `discarded` object when the level left something
out:

```json
{"workday_id":"...","date":"2026-08-24","pauses":0,"tasks":0,"deleted_tasks":0,
 "privacy_level":"coarse","discarded":{"pauses":2,"tasks":1}}
```

Told "5 pauses accepted" by a server that stored none, an agent would report a
break as recorded when it was not.

**Changing the level does not rewrite history.** Narrowing it stops new detail
from arriving and leaves what is already stored; widening it does not bring back
what was dropped. A day the agent re-sends, though, is stored under the level in
force now.

There is no per-employee opt-out. It was considered and rejected: a manager
comparing a team where one person's pauses are missing gets a dashboard that
lies by omission. The unit of the promise is the installation. The reasoning is
in [ADR 0011](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0011-the-privacy-manifest.md).
