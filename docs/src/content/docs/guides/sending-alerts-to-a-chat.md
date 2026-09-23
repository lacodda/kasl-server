---
title: Sending alerts to a chat
description: Point the server at a Slack, Mattermost or Telegram chat - or a system of your own - and it says what it noticed as it notices it.
---

The dashboard's alerts wait for somebody to open the page. A webhook takes the
same alerts to where a manager already is: a team channel hears that an agent
went quiet, that somebody looked, and that it came back - as it happens, not on
Thursday.

Each destination is one environment variable. The name after
`KASL_WEBHOOK_` becomes its label everywhere else - in the log, on the
**Webhooks** screen, in the privacy manifest:

```ini
# .env next to docker-compose.yml
KASL_PUBLIC_URL=https://kasl.example.com
KASL_WEBHOOK_TEAM=slack https://hooks.slack.com/services/T000/B000/XXXXXXXX
```

Restart the server, and the log names what it will send to - by label and host,
never by address:

```console
$ docker compose up -d && docker compose logs server | grep webhook
INFO kasl_server: webhook destination destination=team kind=Slack target=hooks.slack.com events=alert.raised,alert.acknowledged,alert.resolved department=everyone
```

Then open **Webhooks** in the web UI (administrators only) and press **Send a
test**. The message arrives within a few seconds and names what the channel
will hear.

`KASL_PUBLIC_URL` is optional. With it, every message ends in a link to the
person it is about; without it, the message says what happened and leaves the
finding to the reader.

## The four kinds

**Slack.** Create an app with *Incoming Webhooks* turned on, add a webhook to
the channel, and copy its URL:

```ini
KASL_WEBHOOK_TEAM=slack https://hooks.slack.com/services/T000/B000/XXXXXXXX
```

**Mattermost.** *Integrations → Incoming Webhooks → Add*, pick the channel,
copy the URL. Plain `http://` is accepted for a Mattermost on the office
network:

```ini
KASL_WEBHOOK_TEAM=mattermost https://chat.example.com/hooks/xxxxxxxxxxxxxxxxxxxxxxxxxx
```

**Telegram.** Create a bot with [@BotFather](https://t.me/BotFather) and add it
to the group. The target is the bot token; `chat=` is the group's id, which
starts with `-100` for a supergroup:

```ini
KASL_WEBHOOK_OPS=telegram 123456789:AAE-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx chat=-1001234567890
```

**Your own system.** `json` posts the event itself, signed with a secret you
choose - for a payroll system, an incident tool, anything that is not a chat.
The secret is required: an unsigned body is one anybody who learns the address
can forge.

```ini
KASL_WEBHOOK_PAYROLL=json https://payroll.example.com/kasl secret=a-long-random-string events=day.closed
```

What it receives, and how to check the signature, is in
[Webhooks](/kasl-server/reference/webhooks/).

## What a destination hears

Without options, a destination hears the life of an alert: raised,
acknowledged, resolved. Two options narrow or widen that:

- **`events=`** - a comma-separated list of `alert.raised`,
  `alert.acknowledged`, `alert.resolved` and `day.closed`. `day.closed` is
  never on by default: a day closes for every person every day, and a channel
  that hears that should have asked for it.
- **`department=`** - only people in that department, matched without regard
  to case. Quote a name with a space in it:
  `department="Customer Success"`. A channel for one department hears nothing
  about anybody else - including people with no department - which is the same
  boundary the dashboard draws around a manager.

```ini
KASL_WEBHOOK_DESIGN=slack https://hooks.slack.com/services/T000/B111/YYYY department=Design
KASL_WEBHOOK_LEADS=telegram 123456789:AAE-xxxx chat=-1001234567890 events=alert.raised
```

A department that does not exist is not an error at startup - it may be
created later - but the **Webhooks** screen marks it, because until it exists
that destination hears nothing at all.

## When a message does not arrive

The **Webhooks** screen shows each destination's last failure and the last
fifty deliveries. What the dispatcher does:

- **A refusal is final.** A `4xx` - the hook was deleted, the bot was removed
  from the group, the token was revoked - is given up on at once, with the
  receiver's reason kept (`404 no_service`, `403 bot was kicked`). Fix the
  variable and restart; later events go through.
- **A bad moment is retried.** A `5xx`, a timeout, a refused connection, a
  `429` are tried again after 30 seconds, then 2 minutes, 10, 30, an hour, 3,
  6 and 12 - about a day in all - and then given up on in writing.
- **Order is kept per destination.** While one message is waiting for its
  retry, the ones after it wait too, so "resolved" never arrives before
  "raised". Other destinations are not held up.
- **A removed destination is closed out.** Take a variable away and restart,
  and whatever was still queued for it is marked as given up, rather than
  shown as "still trying" forever.

The queue lives in the database, so a restart loses nothing: an alert and its
message are written in one transaction, and the dispatcher picks up where it
left off.

## What employees are told

The privacy manifest lists every destination under `sent_elsewhere` - where,
about whom, and what each message carries - built from the same variables the
dispatcher sends through. See
[What the server stores about you](/kasl-server/concepts/what-the-server-stores-about-you/).
