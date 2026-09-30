---
title: Notes on a day
description: A manager's word on one of a person's days - writing it, reading it beside the days, and withdrawing it.
---

"Your day off on Friday is approved." "Thanks for staying for the release." A
note is a manager's word on one of a person's days, kept on that day and told
to the person on their machine and in their inbox. It goes one way: there are
no replies, and it is not a chat.

## Writing one

Whoever may see a person's days may write on them - the manager of their
department, or an administrator:

```console
$ curl -X POST -H "Cookie: kasl_session=..." -H "Content-Type: application/json" \
    -d '{"date":"2026-10-09","text":"Your day off on Friday is approved."}' \
    http://127.0.0.1:8080/api/v1/users/9dce4dd0-.../notes
{"id":"3b1f...","date":"2026-10-09","text":"Your day off on Friday is approved.",
 "author_id":"85440341-...","author":"Priya Raman","created_at":"2026-09-30T10:04:11Z"}
```

`date` is the person's own calendar date, the one their days are filed under -
and it does not need a day behind it. Leave approved ahead of time is on a date
nothing has been recorded for yet, and the note is on it anyway.

The person is told in the same transaction: a notice of kind `note.added`,
whose sentence is the note itself. See [what you are told](/kasl-server/reference/what-you-are-told/).

| Refused with | When |
| --- | --- |
| `400` | the text is empty, or longer than 1000 characters; the date is more than a year ahead; the day is your own |
| `403` | an employee - nobody writes on their own or a colleague's days |
| `404` | the person is not somebody you can see - the same answer as a person who does not exist |
| `409` | the account is deactivated, so nobody would read it |

The text is stored as sent, less the spaces around it. It cannot be edited
afterwards: the person may already have read it, and a correction is a second
note.

## Reading them

Beside the days, in the answer to [`/me/days`](/kasl-server/reference/reading-your-own-days/)
and to `/users/{id}/days` alike - the notes on dates in the range, oldest
first:

```console
$ curl -H "Cookie: kasl_session=..." "http://127.0.0.1:8080/api/v1/me/days?from=2026-10-05&to=2026-10-11"
{"from":"2026-10-05","to":"2026-10-11","days":[],
 "notes":[{"id":"3b1f...","date":"2026-10-09","text":"Your day off on Friday is approved.",
           "author_id":"85440341-...","author":"Priya Raman","created_at":"2026-09-30T10:04:11Z"}],
 ...}
```

Beside rather than inside the days, because a note's date may have no day.
Whoever can read the days reads the notes on them: the person, their manager,
an administrator.

## Withdrawing one

```console
$ curl -X DELETE -H "Cookie: kasl_session=..." http://127.0.0.1:8080/api/v1/notes/3b1f...
{"id":"3b1f...","withdrawn_at":"2026-09-30T10:06:40Z"}
```

Whoever wrote it, or an administrator. Another manager who can see the day gets
`403`; somebody who cannot, `404`. A second withdrawal is `409`.

**Withdrawing takes the words out.** The note leaves the day, and its text is
gone from the server - a note written on the wrong person's day may say
something they were never meant to read. The notice that announced it stays in
the person's inbox, marked withdrawn and saying only that, so somebody who read
the toast does not find it vanished without a word; a machine that had not
shown it yet does not. Nothing new is told.

## What else keeps a record

The [audit log](/kasl-server/reference/the-audit-log/) records `note.added` and
`note.withdrawn` against the person whose day it is, with the note's id and
date - never its words, because nothing may be deleted from the audit log and a
withdrawal has to reach every copy. Notes are in the backup, and listed in
[the privacy manifest](/kasl-server/concepts/what-the-server-stores-about-you/)
at every level: the level governs what the agent sends, and a note is a
manager's words. They are never sent to a [webhook](/kasl-server/reference/webhooks/).

The reasoning is in [ADR 0021](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0021-notes-on-a-day.md).
