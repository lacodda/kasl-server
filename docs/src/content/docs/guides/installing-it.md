---
title: Installing it
description: Docker and nothing else - no Rust, no Node, no build - plus backups.
---

Docker and nothing else — no Rust, no Node, no build:

```console
$ curl -o docker-compose.yml \
    https://raw.githubusercontent.com/lacodda/kasl-server/main/docker-compose.install.yml
$ printf 'POSTGRES_PASSWORD=%s\n' "$(openssl rand -base64 24)" > .env
$ chmod 600 .env
$ docker compose up -d
$ docker compose logs server | grep -A 4 'administrator account'

  An administrator account was created, because this installation had none:

      email:    admin@kasl.local
      password: 6fp35gainu7zpj2yfwy3
```

Open `http://localhost:8080`, sign in with that, and change it. **The password
is printed once and stored nowhere else** — the alternative, writing one into a
file the server reads at every boot, leaves a credential lying around forever.
To name the administrator yourself instead, set `KASL_ADMIN=email:password`
before the first start.

Nothing to look at yet? `KASL_DEMO=true docker compose up -d` on an empty
database seeds a fictional team — see [The demo](/kasl-server/guides/the-demo/).

The image is `ghcr.io/lacodda/kasl-server`, built for amd64 and arm64, so the
same compose file works on a laptop and on a Raspberry Pi. Pin a version in
production (`KASL_VERSION=0.20.0`); `latest` is for a first look.

Two more things before this holds a team's hours:

- **Put it behind HTTPS** and set `KASL_SECURE_COOKIES=true`. Over plain
  `http://` the session cookie cannot carry `Secure`, which is why the compose
  file ships with it off — convenient for a trial, wrong for anything real.
- **Take backups.** See below; the database's volume is the only copy until you
  do.

## Building from source instead

`docker-compose.prod.yml` builds the image on the machine that will run it,
which is what this project's own stand does — it makes every deploy a proof
that the code compiles for that architecture. It takes about fifteen minutes on
a Pi 4 the first time; later builds reuse the cached layers.

```console
$ docker compose -f docker-compose.prod.yml up -d --build
```

In both cases the database publishes no port — only the server reaches it, over
the compose network — and the server runs as an unprivileged user.

## Backups

The whole installation goes into one file and comes back out of it:

```console
$ docker compose exec server kasl-server backup > kasl-$(date +%F).jsonl
wrote 4213 rows from 12 tables

$ docker compose exec -T server kasl-server restore < kasl-2026-08-28.jsonl
restored 4213 rows into 12 tables
```

The file is JSON Lines — one line per table — so it reads in any tool and
compresses well. `--out` and `--from` take paths instead of the standard
streams.

**A restore refuses a database that already holds accounts.** Merging two
installations would mean deciding what wins, and every answer to that is wrong
for somebody; restore into an empty database and the decision stays yours.

**And it refuses a backup from a newer server.** A file carries the schema
version it was taken at, because the failure being avoided is not an error — it
is a restore that appears to work while quietly dropping columns the older
schema has no place for.

Agent tokens survive a restore, so the machines in the field keep reporting
without being re-enrolled — which matters most on the day you actually need
this. If you already have a backup regime, `pg_dump` remains available and this
does not replace it.
