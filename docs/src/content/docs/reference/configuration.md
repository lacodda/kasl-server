---
title: Configuration
description: Everything the server reads from the environment.
---

Everything comes from the environment:

| Variable | Meaning | Default |
| --- | --- | --- |
| `DATABASE_URL` | PostgreSQL connection string | required |
| `KASL_SERVER_ADDR` | Address the HTTP server binds to | `0.0.0.0:8080` |
| `KASL_AGENTS` | Agents to provision on startup, as `email:token` pairs separated by commas | none |
| `KASL_ADMIN` | First administrator, as `email:password`. Unset, the server generates one on a first run | none |
| `KASL_ADMIN_EMAIL` | Email for that generated administrator | `admin@kasl.local` |
| `KASL_SECURE_COOKIES` | Whether the session cookie carries `Secure`. Set `false` only when serving plain `http://` | `true` |
| `KASL_DEMO` | Seed a fictional team on an empty database, and refuse to start on one that holds real accounts. See [The demo](/kasl-server/guides/the-demo/) | `false` |
| `KASL_MAX_BATCH_DAYS` | Days one `/days/batch` request may carry | `31` |
| `KASL_MAX_BODY_BYTES` | Largest request body accepted | `4194304` |
| `KASL_WEBHOOK_<NAME>` | One destination for events - a Slack, Mattermost or Telegram chat, or a signed JSON receiver. One variable each; `<NAME>` becomes its label. See [Sending alerts to a chat](/kasl-server/guides/sending-alerts-to-a-chat/) | none |
| `KASL_PUBLIC_URL` | The address people open the web UI at. When set, a chat message links to the person it is about | none |
| `RUST_LOG` | Log filter (tracing syntax) | `kasl_server=info,tower_http=info` |

Database migrations are embedded in the binary and applied on startup.

A webhook destination is here and not in the web UI on purpose. A Slack hook
or a Telegram bot token is a working credential to post as somebody; it belongs
next to the database password, not in the database that `kasl-server backup`
writes to a file. A value that does not parse stops the server from starting,
naming the variable and never repeating its value.

The privacy level is deliberately not here. It is set through the API and the
change is audited; an operator editing a file and restarting leaves nothing
behind that says who loosened the policy or when.

`KASL_AGENTS` is how the first agents get in while the admin UI does not exist
yet: each entry becomes an employee and an agent holding that token's hash.
Re-running with a changed token rotates it and revokes the old one. Tokens are
secrets — pass them through your deployment's secret store, not a committed
file — and the variable stops being the way in once tokens are issued from the
UI.
