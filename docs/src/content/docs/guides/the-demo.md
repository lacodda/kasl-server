---
title: The demo
description: Seed a fictional team so the dashboards can be seen before an agent is installed on anybody's machine.
---

A fresh server has nothing on it, and an empty dashboard is where most trials
end. `KASL_DEMO=true` fills an empty database with a fictional team — three
departments, twelve people, eight weeks of days — so the dashboards can be seen
before an agent is installed on anybody's machine:

```console
$ KASL_DEMO=true docker compose up -d        # or: KASL_DEMO=true cargo run
$ docker compose logs server
2026-08-29T18:14:37.667958Z  INFO kasl_server: database schema is up to date version=20260830000001
2026-08-29T18:14:39.401685Z  INFO kasl_server: seeded the demo team people=12 departments=3 days=386

  This is a demo: a fictional team, nothing here is real. Sign in as

      manager   priya.raman@example.com          Priya Raman
      employee  tomas.verhoeven@example.com      Tomas Verhoeven
      admin     sam.whitfield@example.com        Sam Whitfield

  with the password `kasl-demo`. The same password opens every account.

$ curl http://127.0.0.1:8080/health
{"database":"ok","demo":true,"status":"ok","version":"0.20.0"}
```

The login screen offers the same three accounts as buttons, and every screen
carries a banner saying the data is invented.

The days are shaped so that everything the dashboard knows how to show is on
it at once: someone steady, someone working ten-hour days, someone whose hours
shrink week by week, a day open right now, an agent that went silent a week
ago, one that never reported, and an administrator with no agent at all. The
history ends yesterday whenever you start it, and two demos started on the same
day show the same numbers, so a screenshot can be reproduced.

The [live status](/kasl-server/reference/who-is-working-now/) is on it too: somebody working, somebody
on a break, agents idle between days, and two machines that have stopped
answering. A demo is seeded once but a pulse is believed for three minutes, so
the demo re-stamps its own — keeping the stopped agents stopped, and leaving
alone any real kasl pointed at it. A demo seeded before the pulse existed gets
its own on the next start, so upgrading the image is enough.

**The team keeps reaching today.** The history is generated from the day it
is seeded, and nothing moves it afterwards — so a stand left running for a
fortnight would show a team that stopped working a fortnight ago, every row
reading "no days recorded for 16 days". That is a truthful description of the
data and a false one of the product. When the newest day falls more than two
days behind, the next start throws the fictional team away and generates it
again from today. Two days rather than one, so an ordinary weekend is not
staleness: this team works weekdays, and on a Sunday its newest day is rightly
Friday's.

Regenerating loses whatever a visitor clicked — an alert they dismissed, a
password they changed. On a demo that is not a loss, and it is also what keeps
the stand current for free: whatever a later version adds to the seed appears
here by itself, because the stand is not mended field by field, it is born
again.

**The demo refuses a database that already holds accounts.** Twelve invented
people alongside a real team, with nothing to say which rows are which, is the
one outcome worse than no demo — so a flag left in a file after a trial stops
the server with a message rather than turning the installation into one. A
database the demo itself seeded starts normally, with or without the flag, and
keeps its banner: the mark lives in the database, not in the environment.

The agents' tokens are `demo-<firstname>` — `demo-tomas`, for instance — so a
real kasl can be pointed at the demo and its days appear next to the invented
ones. Every address is under `example.com`, which is reserved: the names are
made up, and the domain guarantees the addresses are too.
