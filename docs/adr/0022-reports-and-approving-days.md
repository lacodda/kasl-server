# 0022. Reports, and approving days

Date: 2026-10-06

Status: Accepted

## Context

Some teams sign off on time. The hours a person worked feed a timesheet, an
invoice to a client, a payroll run, and somebody has to say "yes, that week is
right" before they do. Today that happens outside the server - an export, a
spreadsheet, an email - and the server, which holds the hours, holds nothing
about whether anybody agreed to them.

The person's half of this already exists, in kasl: `kasl report --send` closes
the day and sends its report to whoever the company has it go to. The first
schema anticipated the server's half with an empty `reports` table - "a report
is an event, not a second copy of the day ... what approval and the late /
missing report signals are built on" - and nothing ever wrote to it.

Three things make this harder than a flag on a day.

**The day is kasl's, not the server's.** The last upload wins (ADR 0004): a
person corrects a day in kasl and the correction replaces what the server
holds. An approval cannot freeze the day - refusing an upload because a manager
answered would leave kasl and the server disagreeing about the same day, with an
agent in the wild that has never heard of the refusal retrying it.

**An approval is of figures.** "Tuesday is approved" means "seven and a half
hours on Tuesday, nine to five, is approved". If kasl later sends Tuesday as
eight hours, the approval was not of that.

**Not every team wants it.** An installation that never asked for approval must
not grow a queue of days waiting for somebody.

## Decision

**A report is the person saying a day is finished, at the figures it came to.**
`POST /api/v1/me/reports` from the web, `POST /api/v1/agent/reports` from kasl -
the same act, written by the same code. Only a day that exists and is closed
can be reported: an open day has no total to put a name to. The figures are
copied into the report - kind, start, end, hours worked - because they are
exactly what a later upload can no longer show.

**A report is an event, never edited.** Reported again after a correction, a
day has a second report; the first stays, with whatever a manager said about
it. What a day's report says now is its newest one. Sending the same day twice
while its newest report still stands for the same figures answers that report
again rather than writing a copy: an agent that lost the answer to the network
retries without filling a manager's queue.

**The status is derived, never stored.** A day's report is `submitted`,
`approved`, `returned`, or `changed` - and `changed` is not written by anybody.
It is the comparison between the report's figures and what the day comes to now,
made on every read. A day re-sent unchanged (a retry, a backfill) still matches,
and stays approved; a day corrected after its approval no longer does, and
reads as `changed` until the person reports it again. Uploads are never refused,
the approval is never silently stretched over figures nobody saw, and no write
path has to remember to update a status - there is none to update.

The comparison needs one definition of what a day comes to, on both sides of it.
That is a view, `workday_figures`: the span minus what was paused, stored pauses
where they exist and the day's own totals where a narrower policy summarized them
away (ADR 0011), whole seconds rounded down. The days answer computes the same
figure in Rust, and a test holds the two to each other on the shapes where they
could part - fractional seconds, a pause the agent did not measure, a coarse
policy.

**Approval is opt-in, per installation.** `settings.day_approval`, set by an
administrator (`PUT /api/v1/reports/approval`), off by default. Off, a report is
only what the person said; on, it is also a question to their manager. Turning
it off answers nothing and erases nothing - approvals stand as given, and reports
nobody answered stop waiting until it is on again. Per installation rather than
per department: whether hours are signed off is how a company pays people, and
two departments of one company paying on different rules is a case nobody has
asked for yet.

**Who answers is who may see the day, never its person.** The visibility rule
the team screens already apply (ADR 0009), asked of the database; a report on a
day the reviewer cannot see does not exist to them. Nobody approves or returns
their own day - a manager sees their own days, so the rule alone would let them -
and a constraint holds that as well as the handler. A manager's own days are
their manager's, or an administrator's.

**Two answers.** Approve, for any number of reports at once
(`POST /api/v1/reports/approve` takes a list): "approve the week" is what a
manager does on a Friday, and each report in the list is decided on its own, the
way a day in an upload batch is (ADR 0005). Return, for one, with a reason that
is required - the person has to know what to look at. Only the newest report of
a day can be answered, and only while it still stands for the day. A returned
report stays returned whatever the day does next; the person reports it again,
unchanged if they think it is right. An approved report can still be returned:
a manager who notices on Monday what they approved on Friday has to be able to
say so.

**The person is told, once per act.** `report.approved` names who approved and
which days at which figures - one notice for a whole approval, because five
toasts saying the same thing teach a person to stop reading them (ADR 0020).
`report.returned` names who and which day, and its sentence is the reason. The
reason is kept once, on the report, and read from there when the notice is - the
rule the words of a note follow (ADR 0021). The audit log records each approval
and return against the person, with the report and the date, never the reason.

**What waits** is `GET /api/v1/team/reports`: the newest report of every day of
everybody the reader may see, unanswered and still standing for its day, oldest
first. A report whose day moved is not waiting for a manager; it is waiting for
its person.

## Consequences

The old `reports` table and `report_kind` are dropped by the migration that
creates the new ones. Nothing ever wrote to them; a table somebody filled by
hand is refused rather than dropped, because losing rows nobody can name is the
one choice a migration must not make silently. The monthly kind goes with it:
closing a pay period is its own milestone.

The days answer gains `report` on each day and `day_approval` beside them,
additive like every change to a v1 response. `notification_kind` gains
`report.approved` and `report.returned`; an agent that does not know them shows
the sentence. `POST /api/v1/agent/reports` has no client yet - kasl's
`report --send` is where it belongs - and is exercised by this repository's
tests until it does.

Reports are in the backup and in the privacy manifest at every level: a report is
the person's word, not something the agent measured, and the level does not
govern it. They are not sent to webhooks; an approval feeding another system is
a later decision with its own reader to design for.

The older SQL copies of "hours worked" still compute it themselves. The team
table, the heatmap and the signals follow the same rule but round fractional
seconds rather than dropping them, so they may differ from the days answer by a
second. The alert sweep's overwork figure follows another rule altogether -
pauses measured from their ends, and the totals a coarse policy keeps not read
at all, so under `coarse` it takes a day's span for its hours. All four belong
on `workday_figures`; that is recorded as the next step rather than done here.
