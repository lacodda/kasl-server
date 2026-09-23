# 0019. Webhooks: where they are declared, and how they are delivered

Date: 2026-09-23

Status: Accepted

## Context

ADR 0018 made the server notice things on its own and then left them on a
page: "Webhooks into a chat are the next milestone (v0.23) ... the row written
here is precisely what those will ship outward. Delivery gets added to the
record. It does not replace it."

The plan named the events as "a day closed, a report arrived, an alert" and
the targets as Slack, Telegram and Mattermost. Three questions had to be
answered before any of it could be written, and each has a cheap answer that
would have had to be undone.

## Decision

**Destinations are declared in the environment, one variable each
(`KASL_WEBHOOK_<NAME>`), and never stored in the database.** A Slack hook URL
and a Telegram bot token are working credentials to post as somebody. This
project's rule for credentials is that they live in the deployment's
configuration and that the database holds only hashes (ADR 0008); a hook
cannot be hashed, because the server has to use it. The database is also what
`kasl-server backup` writes to a plain file, and a backup that hands out a
working hook to whoever finds it is a leak with a delay. So the address stays
beside the database password, and everything else - the delivery log, the
**Webhooks** screen, the privacy manifest, the startup log - knows a destination
by the label its variable gives it.

The cost is that an administrator cannot add a destination from the web UI.
The screen shows what is configured, how the last deliveries went, and sends a
test; adding one is an edit to the deployment and a restart. That is the same
place the first administrator and the agent tokens were bootstrapped, and the
same kind of act.

A value that does not parse stops the server from starting. A destination
dropped because of a typo is a channel that never hears about the agent that
died, and nothing would say so. Errors name the variable and never repeat the
value.

**An event is queued in the transaction that made it true, and delivered from
the database.** The alert row and its delivery row commit together; so do an
uploaded day and its `day.closed`. The alternative - an in-memory channel from
the sweep to a sender - loses everything queued on every restart, and "the
server restarted just as the agent died" is exactly the night nobody would
hear about. The table `webhook_deliveries` holds one row per event per
destination, with its attempts, its next attempt, and how it ended.

Event ids are derived from what the event is about (an alert and the step it
took; a day and when it ended) rather than random, and `(event_id,
destination)` is unique. Two sweeps racing each other, or an upload retried,
queue the same fact once.

**Delivery is at least once, in order per destination, and given up on in
writing.** A dispatcher looks every five seconds and, per destination, sends
the oldest thing in flight and then the next while they succeed. A failure
stops that destination until its retry is due, so later events wait behind it:
"resolved" must not arrive in a channel before "raised". Retries run 30 s,
2 m, 10 m, 30 m, 1 h, 3 h, 6 h, 12 h - about a day - and then the row is marked
abandoned with the reason. A `4xx` other than `408`/`429` is a refusal (the hook
was deleted, the bot was kicked) and is abandoned at once; retrying it for a day
would only delay the moment somebody notices. A delivery queued for a
destination whose variable has since been removed is abandoned the same way,
rather than showing as pending forever.

Every event carries its id, and a request that timed out may still have
arrived, so receivers are told plainly to deduplicate.

**What is sent.** The events are the life of an alert (`alert.raised`,
`alert.acknowledged`, `alert.resolved`), `day.closed`, and `test`. Resolution
is sent even for an acknowledged alert: the channel was told it was raised and
that somebody looked, and "it is over" is what lets it stop wondering.
Acknowledgement carries who answered, so two managers reading the same message
do not both chase it.

`day.closed` is announced once, when a day first arrives finished, and only if
it ended within the last day. An agent back from a fortnight offline sends a
fortnight of finished days at once, and ten messages about days everybody has
forgotten are noise; a correction to when a day ended is not a second close.

"A report arrived" is **not** an event, because nothing in this server receives
a report: the `reports` table has existed since the core schema and no route
writes to it. The event joins the list when a route does.

A destination hears the alert events by default and `day.closed` only when it
asks - a day closes for every person every day. `department=` restricts it to
one department, and to nobody else, which is the boundary every screen draws
around a manager (ADR 0009).

**Four kinds, one event.** `slack`, `mattermost` and `telegram` render the event
as a message in each one's own markup, with each one's own escaping - a display
name is typed by a person. `json` posts the event itself, signed with
HMAC-SHA256 over `<timestamp>.<body>`, and the secret is required: an unsigned
body is one anybody who learns the address can forge. The chat text is rendered
from the same payload the `json` kind receives, so a message cannot say what
the payload does not.

**The privacy manifest lists every destination.** Under `sent_elsewhere`: where
(by kind and label), about whom, and what each message carries - generated from
the configuration the dispatcher sends through (ADR 0011). A manifest that
described storage and said nothing about a channel receiving somebody's hours
would describe a quieter server than the one running.

## Consequences

The HTTP client is reqwest on rustls with the `ring` provider and the Mozilla
roots compiled in - the TLS stack sqlx already links - rather than reqwest's
default, which brings aws-lc-rs and a C toolchain to every build. Redirects are
not followed: a hook that redirects has moved, and following it sends the body
somewhere nobody configured.

Errors are stored without the address. reqwest's own error text names the URL
it was sending to, so the dispatcher strips it at the source and then redacts
every credential the destination holds from whatever text it keeps - the
second lock is for the text nobody predicted.

One dispatcher per database is assumed, as one sweep is. Two servers against
one database would both send the same due row; the unique event id lets a
receiver discard the duplicate, but running two is not a supported layout.

Delivered rows are kept, like alerts: "was the channel told" is a question the
log answers months later. They name people, so the screen that shows them is
for administrators only.
