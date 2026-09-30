# 0021. Notes on a day

Date: 2026-09-30

Status: Accepted

## Context

A manager has a few things to say about somebody's time that are not about the
numbers: "your day off on Friday is approved", "thanks for staying for the
release", "leaving early for the dentist is fine". Today they travel through a
chat, detached from the day they are about, and are found again by scrolling.
The day itself - the one thing both people look at - carries nothing.

The server already has the two halves this needs. The drill-down is one
person's days, shown to whoever may see them (ADR 0009). The channel to the
employee exists since 0.24 and was built for exactly this: "a manager's note on
a day is a new value of `notification_kind` and a sentence, not a new route"
(ADR 0020).

What it must not become is a chat. The product measures time; a thread under
every Tuesday would make it a messenger that also measures time, and would put
a conversation about a person in the one place that person's hours are kept.

## Decision

**A note is a manager's word on one of a person's days, and the person is
told.** Written by whoever may see the day - the visibility rule the team
screens already apply, asked of the database rather than restated - to the
person whose day it is. It goes one way. There are no replies.

**On a date, not on a workday.** The note most worth writing is about a day
that has not happened: leave approved ahead of time has no workday row, and
neither has a day the agent never reported. A note is keyed the way the
person's own days are - who, and their local calendar date - in its own table,
`day_notes`, and lands on the same line of the week whether or not kasl ever
sends that day. The days answer carries the range's notes beside the days
rather than inside them, for the same reason.

**Written once.** A note cannot be edited. The person may already have read it
- as a toast on their machine, in the inbox - and words that change under them
are words they cannot rely on. A correction is a second note.

**Withdrawn, not deleted, and withdrawing takes the words out.** A note on the
wrong person's day must be taken back, and taking it back has to mean the text
is gone - it may say something about somebody who was never meant to read it.
So the note's row keeps who, when and on which day, and loses its text; the
notice that announced it stays in the inbox as withdrawn, so a person who read
the toast does not find it vanished without a word. Nothing new is told: "your
manager took a note back" is not something to act on.

For that to be true there can be one copy of the words. The notice's payload
names the note, its date and its author, and reads the text from the note when
it is read - the same way it reads whether an alert is still true from the
alert rather than from a copy (ADR 0020). Because a note cannot be edited, what
is read is always what was told. The audit log records that a note was written
and withdrawn, on whose day and by whom, and never the words: it is the one
table nothing may delete from (ADR 0010), and a withdrawal that left the text
there would be a withdrawal in name only.

**Who may take it back: whoever wrote it, or an administrator.** Another
manager who sees the day is told why not; somebody who cannot see the day is
answered as if the note did not exist. The person whose day it is cannot
withdraw what was said to them.

**Not on your own day.** A manager sees their own days, so the visibility rule
alone would let them write there - and the notice would go to its own author.
Refused, and held by a constraint as well as the handler.

**Bounds.** A thousand characters: a note, not a letter; what does not fit in a
toast belongs in a conversation. Dated at most a year ahead: leave is approved
months out, and two years out is a mistyped year. Any past date: history is
imported years back. Not on a deactivated account, which has nobody left to
read it.

**Not sent outward.** Webhooks carry what the server noticed to a team's chat
(ADR 0019); a note is between a manager and one person, and posting it to a
channel would be the opposite of why it is written here.

## Consequences

`POST /api/v1/users/{id}/notes` and `DELETE /api/v1/notes/{id}`. The days
answer - `/me/days` and `/users/{id}/days` alike - gains `notes`, additive like
every change to a v1 response. `notification_kind` gains `note.added`; an
agent that does not know it shows the sentence, which is the note itself.

A notice about one day now links to that day: `/day?date=2026-10-02` opens its
week with it expanded. Alerts about a day follow the same rule.

The notes are listed in the privacy manifest at every level. The level governs
what the agent sends, and a note is not that - it is a manager's words, kept
whatever the level, and a manifest without them would describe a quieter
server than the one running. `day_notes` is in the backup; without it every
notice about a note would restore as withdrawn.

The demo carries three notes, one of them on a day to come for the employee a
visitor is offered to sign in as. A demo stand upgraded from 0.24 would have
had none - the third field in a row the upgrade path would have had to patch
in (the pulse in 0.17.1, the calendar in 0.21). Instead a demo records the
version that generated it and is generated again whenever another version
starts it, so whatever a milestone adds to the seed reaches the stand with the
image (ADR 0013).

The paired agent milestone is kasl v3.3, the same as for the channel itself:
the toast shows the note as the sentence, and nothing more is needed to read
one.
