<p align="center"><img src="https://github.com/lacodda/kasl-server/raw/main/assets/banner.svg" alt="kasl-server - team server for kasl" width="720"></p>

# kasl-server

Team server for [kasl](https://github.com/lacodda/kasl). Employees run kasl on their machines; the agents send work-time data to the server. Managers get dashboards, charts, and reports across the whole team; every employee gets a personal page.

> **Status: pre-alpha.** The door for kasl agents is open and survives a bad connection: a day at a time on `POST /api/v1/days`, a backlog on `/days/batch`, and a task the employee deleted can be deleted here too. History from before the server arrived can be imported from an agent's own database; people sign in, and an administrator manages the team, its departments and its agent tokens without touching the host, and every such change is recorded. What the server keeps about a person is now a policy it enforces rather than a claim: an employee can ask it, and an administrator can narrow it. The loop is closed: agents deliver, employees see their own week, and a manager sees their team's - hours per person, a status that says what the server actually knows, and a drill-down into anyone's days. The same binary serves all of it, and installing it is a compose file and a published image rather than a build. What is still missing is the other half of the loop: kasl cannot send on its own yet, so history is imported or posted by hand.

## Try it

Requires Rust and Docker.

```console
$ git clone https://github.com/lacodda/kasl-server && cd kasl-server
$ docker compose up -d db
$ export DATABASE_URL=postgres://kasl:kasl@localhost:5433/kasl
$ export KASL_AGENTS=employee@example.com:agent-token
$ cargo run
2026-08-28T11:48:44.511775Z  INFO kasl_server: database schema is up to date version=20260826000001
2026-08-28T11:48:44.758115Z  INFO kasl_server::provision: provisioned agents from KASL_AGENTS agents=1
2026-08-28T11:48:44.792230Z  INFO kasl_server: kasl-server listening version="0.13.0" addr=0.0.0.0:8080 max_batch_days=31 max_body_bytes=4194304

$ curl http://127.0.0.1:8080/health
{"database":"ok","status":"ok","version":"0.13.0"}

# The web UI is served by the same binary on the same port - open
# http://127.0.0.1:8080 and sign in.

# An agent back from three days offline. The middle day is impossible - it ends
# before it starts - and the others land anyway.
$ curl -X POST http://127.0.0.1:8080/api/v1/days/batch \
    -H "Authorization: Bearer agent-token" -H "Content-Type: application/json" \
    -d '{"days":[
         {"date":"2026-08-15","started_at":"2026-08-15T09:04:00-03:00","ended_at":"2026-08-15T18:12:00-03:00",
          "tasks":[{"agent_task_id":7,"recorded_at":"2026-08-15T18:10:00-03:00","name":"Reliable ingest","completeness":60}]},
         {"date":"2026-08-16","started_at":"2026-08-16T19:00:00-03:00","ended_at":"2026-08-16T09:00:00-03:00"},
         {"date":"2026-08-17","started_at":"2026-08-17T09:11:00-03:00","ended_at":"2026-08-17T17:40:00-03:00",
          "tasks":[{"agent_task_id":7,"recorded_at":"2026-08-17T17:38:00-03:00","name":"Reliable ingest","completeness":100}],
          "tasks_are_complete":true}]}'
{"accepted":2,"rejected":1,"results":[
  {"status":"accepted","workday_id":"82ca500d-feeb-4d1f-8fb7-0b376339be02","date":"2026-08-15","pauses":0,"tasks":1,"deleted_tasks":0},
  {"status":"rejected","date":"2026-08-16","error":"ended_at is before started_at"},
  {"status":"accepted","workday_id":"2ea46d40-3aa0-48d3-8d8d-e1bb152a36bc","date":"2026-08-17","pauses":0,"tasks":1,"deleted_tasks":0}]}

# The employee deletes the task in kasl; the agent re-sends the day and says so.
$ curl -X POST http://127.0.0.1:8080/api/v1/days \
    -H "Authorization: Bearer agent-token" -H "Content-Type: application/json" \
    -d '{"date":"2026-08-17","started_at":"2026-08-17T09:11:00-03:00","ended_at":"2026-08-17T17:40:00-03:00",
         "tasks":[],"tasks_are_complete":true}'
{"workday_id":"2ea46d40-3aa0-48d3-8d8d-e1bb152a36bc","date":"2026-08-17","pauses":0,"tasks":0,"deleted_tasks":1}
```

The task is gone from the 17th - and still there on the 15th, where the employee
did not delete it.

The dev database listens on 5433, leaving a PostgreSQL you may already run on
5432 alone; override with `KASL_DB_PORT`.

## The API

`/api/v1` from the first endpoint: agents update on their own schedule, so a
path keeps meaning what it meant when the agent calling it shipped.

**`POST /api/v1/days`** — upload one day. Requires `Authorization: Bearer
<token>`. The body is the workday with its pauses and tasks; `ended_at` is
absent while the day is still running, and so is a pause's, and `tasks` may be
empty.

Two properties are worth knowing before writing a client:

- **Every instant needs a UTC offset**, and the day carries its own `date`.
  `2026-08-14T09:12:00` without an offset is refused (422): one team's hours
  have to stay comparable across time zones, and which calendar day work
  belongs to is the agent's call, not a value derived on the server.
- **The last upload wins.** Re-sending a day replaces what is stored, so a
  correction made in kasl lands and a retry after a lost connection is safe -
  the same payload twice leaves the same rows. Pauses are replaced as a set;
  tasks are matched on `agent_task_id`, so a task carried into the next day
  moves rather than multiplying.
- **Deleting a task takes one word.** Send `"tasks_are_complete": true` and the
  date's tasks the payload omits are deleted, which is how a task the employee
  removed in kasl disappears here too. Other dates are untouched. Leave the flag
  out - as agents written before it did - and nothing is ever deleted.

**`POST /api/v1/days/batch`** — upload a backlog. The body is `{"days": [...]}`
with the same day objects, and the answer reports each one:

```json
{"accepted": 2, "rejected": 1, "results": [
  {"status": "accepted", "date": "2026-08-10", "workday_id": "...", "pauses": 1, "tasks": 3, "deleted_tasks": 0, "privacy_level": "full"},
  {"status": "rejected", "date": "2026-08-11", "error": "ended_at is before started_at"},
  {"status": "accepted", "date": "2026-08-12", "workday_id": "...", "pauses": 0, "tasks": 1, "deleted_tasks": 0, "privacy_level": "full"}
]}
```

Each day is written on its own, so one the server will never accept does not
block the rest - an agent that could not deliver *any* of its backlog because of
a single bad row would retry the same request forever. The batch carries at most
`KASL_MAX_BATCH_DAYS` days (31) and the body at most `KASL_MAX_BODY_BYTES`
(4 MiB); past either, `413`.

**Which failures are worth retrying.** `4xx` means the payload will not be
accepted as sent, however many times it is tried - fix it or drop it. `5xx`
means the fault is on this side; send it again later. A batch that answers `5xx`
stopped partway: the days already accepted are stored, and re-sending them is
safe because the last upload wins.

**Every accepted day reports the privacy level that applied**, and a
`discarded` object when that level left something out. See
[what the server stores about you](#what-the-server-stores-about-you): an agent
should never report a break as recorded by a server that did not store it.

A malformed day is refused with `400` and a reason naming the field
(`{"error":"tasks[0]: completeness must be between 0 and 100"}`); an
unrecognized, revoked or deactivated token gets `401`. The reasoning behind all
of this is in [ADR 0004](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0004-the-ingest-contract.md)
and [ADR 0005](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0005-deletions-and-backfill.md).

## Signing in

People sign in with an email and a password; kasl agents keep using their bearer
token and are unaffected by any of this.

```console
$ kasl-server admin --email boss@example.com --password '...'
admin boss@example.com is ready

$ curl -i -X POST http://127.0.0.1:8080/api/v1/auth/login     -H "Content-Type: application/json"     -d '{"email":"boss@example.com","password":"..."}'
HTTP/1.1 200 OK
set-cookie: kasl_session=b7857303342e1a4b...; Path=/; HttpOnly; SameSite=Strict; Max-Age=1209600
{"status":"ok"}

$ curl -H "Cookie: kasl_session=b7857303342e1a4b..." http://127.0.0.1:8080/api/v1/auth/me
{"id":"de836432-5dce-4705-9344-a65b356fc662","email":"boss@example.com","display_name":"boss@example.com","role":"admin"}
```

`POST /auth/logout` ends this session, `POST /auth/logout-everywhere` ends all of
them, and `GET /auth/me` says who the caller is.

Sessions are rows in the database, not signed tokens. The query per request buys
the thing a self-contained token cannot give: access ends when it is ended — the
afternoon someone leaves, not whenever their token happens to expire. A session
lasts a fortnight and each use pushes that out again.

An unknown email, a deactivated account and a wrong password all answer
`{"error":"wrong email or password"}`, so the login form cannot be used to find
out who works somewhere.

**The first administrator** comes from `kasl-server admin` or
`KASL_ADMIN=email:password` in the environment. Running it again resets the
password and promotes the account, which is both the way back in after a
forgotten password and the way to make an admin of someone whose account already
exists because their agent has been reporting. Accounts created by `KASL_AGENTS`
have no password and cannot be signed into — they exist to own an agent's data.

Set `KASL_SECURE_COOKIES=false` when serving over plain `http://`. A `Secure`
cookie is silently dropped by the browser there, which looks exactly like login
doing nothing. The reasoning is in
[ADR 0007](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0007-sessions-and-the-first-admin.md).

## Reading your own days

`GET /api/v1/me/days` answers the signed-in person's own history: the workdays
in a range, each with its pauses and the tasks logged on it. Both ends are
inclusive, so one date twice is one day.

```console
$ curl -H "Cookie: kasl_session=..."        "http://127.0.0.1:8080/api/v1/me/days?from=2026-08-24&to=2026-08-30"
{"from":"2026-08-24","to":"2026-08-30","privacy_level":"full","not_stored":[],
 "days":[{"date":"2026-08-24","started_at":"2026-08-24T12:05:00Z","ended_at":"2026-08-24T21:12:00Z",
          "worked_seconds":29640,"paused_count":2,"paused_seconds":3180,
          "pauses":[{"id":"56e75449-...","started_at":"2026-08-24T15:30:00Z","ended_at":"2026-08-24T16:15:00Z",
                     "duration_seconds":2700,"manual":true,"reason":"lunch"}],
          "tasks":[{"id":"2a9bd688-...","name":"Read-only endpoint","comment":null,
                    "completeness":100,"recorded_at":"2026-08-24T20:50:00Z"}]}]}
```

`worked_seconds` is the day's span minus what was paused, and it is `null` while
the day is still open — a day in progress has no total, and reporting the hours
so far as the day's figure would make every working afternoon look short.
`paused_count` and `paused_seconds` are always answered, whether they come from
the stored pauses or from the totals a coarse policy keeps instead.

**The route is `/me`, not your own id under `/users`.** It consults no role and
no department, so there is no permission here to read wrong; reading someone
else's days arrives with the manager's dashboard as its own route, where the
check is the point. A session is required — an agent's bearer token writes days
and reads the privacy manifest, and that is deliberately the whole list.

**`not_stored` names what the installation's privacy level withholds** —
`pauses`, `tasks`, `free_text`, or nothing at all. It is what lets a screen say
"not stored" where it would otherwise draw an empty section: an employee cannot
tell "no pauses were kept" from "you took no breaks", and only one of those is
true (ADR 0011).

A range covers at most 400 days; a wider one, or one that runs backwards, is a
`400` naming the span asked for rather than a truncated answer.

## The team's hours

`GET /api/v1/team/days` answers a row per person over a range: hours, days
recorded, pauses, and what the server knows about them right now. Managers and
administrators only.

```console
$ curl -H "Cookie: kasl_session=..."        "http://127.0.0.1:8080/api/v1/team/days?from=2026-08-24&to=2026-08-30"
{"from":"2026-08-24","to":"2026-08-30","privacy_level":"full","not_stored":[],
 "members":[
   {"id":"c49ea6a8-...","display_name":"Anna","email":"anna@example.com","department":"Engineering",
    "days_recorded":3,"worked_seconds":72600,"paused_seconds":8100,"last_day":"2026-08-26",
    "day_open":false,"last_seen_at":"2026-08-26T20:31:00Z","agents":1},
   {"id":"0073460d-...","display_name":"Clara","email":"clara@example.com","department":null,
    "days_recorded":0,"worked_seconds":0,"paused_seconds":0,"last_day":null,
    "day_open":false,"last_seen_at":null,"agents":0}]}
```

**Everyone the reader may see is listed, including people with nothing
recorded.** Clara above has no agent installed and no days; she is on the list
anyway, because an employee whose agent never reported is exactly who a manager
needs to notice. A table that dropped her would hide the case it exists for.

**`day_open` and `last_seen_at` are what the server actually knows** - whether
a day is open on that person's own calendar, and when one of their agents last
delivered anything. Neither says someone is at their keyboard; that needs
heartbeats, which is a later milestone. The dashboard says "day open, last data
20 min ago" rather than "working now", because only the first is true.

**Who sees whom** is the rule departments established: an administrator sees
everyone, a manager sees the departments they run plus themselves, and a person
in no department is visible to the administrator alone
([ADR 0009](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0009-departments-and-visibility.md)).

`GET /api/v1/users/{id}/days` is the drill-down: **the same response as
`/me/days`**, for a person the caller is entitled to see. An id they may not
see answers `404` rather than `403` - a manager probing ids should not be able
to tell an employee in another department from one who does not exist.

## Managing the team

Once an administrator exists, people and agent tokens are managed over the API
rather than through the host's environment.

```console
$ curl -X POST http://127.0.0.1:8080/api/v1/users -H "Cookie: kasl_session=..."     -H "Content-Type: application/json"     -d '{"email":"ivan@example.com","display_name":"Ivan","password":"..."}'
{"id":"9b5c1fd8-cf3d-433e-bb9e-0c2bf1c1cfac"}

$ curl -X POST http://127.0.0.1:8080/api/v1/users/9b5c1fd8-.../agents     -H "Cookie: kasl_session=..." -H "Content-Type: application/json"     -d '{"name":"ivan-laptop"}'
{"id":"b749b090-db08-464d-b48d-4fe15f7acc43","name":"ivan-laptop",
 "token":"kasl_<64 hex chars>",
 "notice":"this token is shown once; the server keeps only its hash"}

$ curl -X DELETE http://127.0.0.1:8080/api/v1/agents/b749b090-... -H "Cookie: kasl_session=..."
# 204; the same token now gets 401 from the ingest routes
```

| Route | Who |
| --- | --- |
| `GET /users`, `GET /users/{id}/agents` | admin (everyone), manager (their departments) |
| `GET /departments` | admin, manager |
| `POST /departments`, `PATCH`/`DELETE /departments/{id}` | admin |
| `PUT /users/{id}/department` | admin |
| `POST /users`, `PATCH /users/{id}` | admin |
| `POST /users/{id}/agents`, `DELETE /agents/{id}` | admin |
| `POST /auth/password` | anyone signed in, for their own password |
| `GET /audit` | admin |

**A manager reads their departments and changes nothing.** A department names its
manager, and a person belongs to one:

```console
$ curl -X POST http://127.0.0.1:8080/api/v1/departments -H "Cookie: kasl_session=..."     -H "Content-Type: application/json"     -d '{"name":"Engineering","manager_id":"d7c9ef3a-..."}'
{"id":"997c3947-4028-45e6-9c1c-9cd334b10c5d"}

$ curl -X PUT http://127.0.0.1:8080/api/v1/users/<id>/department -H "Cookie: kasl_session=..."     -H "Content-Type: application/json" -d '{"department_id":"997c3947-..."}'
# 204; `{"department_id":null}` takes them out again without deleting anything
```

The manager of Engineering sees the people in Engineering, plus themselves — a
manager who runs nothing yet would otherwise get an empty page and think the
product was broken. An administrator sees everyone.

**Someone with no department is visible to the administrator alone.** Showing
the unfiled to every manager, so nobody gets lost, fails in the direction nobody
observes: forget to file a person and they are exposed company-wide, silently.
Missing from a list is reported the same afternoon.

Deleting a department leaves its people unfiled rather than deleting them, and
an employee cannot be made to run one — they could not see it, so it would
silently have no working head. Issuing an agent token stays with the
administrator: it is the authority to write someone's history, and there is no
audit log until the next milestone.

**An administrator sets an initial password and hands it over; the person
changes it** with `POST /auth/password`, which requires the current one. The
server has no mail channel, so there is nothing to send an invite link to that
would not be handed over the same way a password is.

Some things follow from a change rather than being asked for separately:

- Deactivating someone, or resetting their password, deletes their sessions.
- Changing your own password ends every *other* session and keeps the one you
  are using.
- The last administrator cannot be demoted or deactivated — the only way back
  from that is the `admin` subcommand on the host.
- A user is never deleted, only deactivated: their days have to keep an owner.

Agent tokens are shown once and stored as a SHA-256. The reasoning is in
[ADR 0008](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0008-roles-and-agent-tokens.md)
and [ADR 0009](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0009-departments-and-visibility.md).

## The web UI

The same binary that answers the API serves the web app, on the same port:
`http://127.0.0.1:8080` is the sign-in screen, and `/api/v1/...` is the API. A
self-hosted install is one file - there is no web server to configure, and no
way for the UI to be from a different build than the API it calls.

**My week** is the employee's own history: seven days, each drawn as a timeline
of gold stretches broken by the pauses in them, and any day opens to its pauses
and the tasks logged on it. **The team** is the manager's dashboard - a row per
person with their hours, bars that compare people with each other, and a status
that says what the server knows; clicking a row opens that person's week in the
same component the personal page uses. **What is stored about me** renders the
manifest from what the server actually enforces (ADR 0011) rather than
describing it again in the page, so the two cannot disagree.

The team screens are hidden from an employee's navigation, but that is tidiness
rather than security: the endpoints behind them refuse an employee outright.

Where the installation's privacy level withheld something, the page says so in
that spot. A `coarse` day draws no timeline and states how many interruptions
there were and how long they came to, because an unbroken gold bar would be a
claim about the day that the server did not keep the evidence for.

The version in the header comes from `/health` - the server's, not the
bundle's. One product, one number.

### Working on it

```console
$ pnpm --dir frontend install
$ pnpm --dir frontend dev      # http://localhost:5173, proxies /api to :8080
```

Vite serves the app and proxies `/api` to the server, so the browser sees one
origin and the session cookie behaves as it will in production. A change is a
page reload rather than a Rust compile.

```console
$ pnpm --dir frontend lint     # eslint, tsc and the unit tests
$ pnpm --dir frontend build    # what gets embedded
```

**TypeScript is held at 6.x on purpose.** typescript-eslint does not run under
TS 7 yet, so upgrading takes linting with it - `pnpm update --latest` pulls 7
and `pnpm lint` then fails before it type-checks anything. The `^6.0.3` range
in `frontend/package.json` is what keeps that from happening by accident.

`cargo build --release` embeds whatever is in `frontend/dist` at that moment.
The repository carries an empty placeholder there, so a checkout without Node
still compiles - and a binary built that way says
`no web UI was built into this binary` instead of serving a blank page. The
decision and its trade-offs are in
[ADR 0012](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0012-serving-the-web-ui.md).

## What the server stores about you

An employee is asked to run an agent that notices when they stop typing. The
honest answer to "what does it send" is one the server enforces and can recite,
so it is an endpoint rather than a paragraph:

```console
$ curl -H "Authorization: Bearer $KASL_TOKEN" http://127.0.0.1:8080/api/v1/privacy/agent
{"level":"full",
 "summary":"This server stores your working hours, every interruption with the reason you gave for it, and the tasks you logged with their comments.",
 "stored":[{"what":"workdays","detail":"the date, when the day started, and when it ended"},
           {"what":"pauses","detail":"each interruption: when it began, how long it lasted, ..."},
           {"what":"tasks","detail":"what you logged: the name, your comment, and how complete you marked it"},
           {"what":"pause reasons","detail":"the text you type when you take a break by hand"},
           {"what":"account","detail":"your email, display name, role, department, ..."}],
 "never_collected":["keystrokes or what you type","window titles","which applications you run",
                    "screenshots or camera images","web pages you visit","file names or paths","your location"],
 "visible_to":["you, in your own account","the manager of your department","administrators of this installation"],
 "retention":"Kept for as long as the installation keeps it: there is no automatic deletion. ...",
 "on_change":"Changing this setting affects what arrives from now on. ..."}
```

An agent reads it with its own token, so kasl can show it in the CLI - where the
employee already is - instead of requiring a login to the server that watches
them. Anyone signed in reads the same manifest at `GET /api/v1/privacy`.

### Three levels

How much detail an installation keeps is a setting, and the server enforces it
on the way in. A field a level excludes is dropped before the day is written: it
never reaches the database, never reaches a backup, and no later change of mind
can recover it.

| Level | Workdays | Pauses | Tasks | Free text |
| --- | --- | --- | --- | --- |
| `full` (default) | hours | each one, timed | name and comment | kept |
| `moderate` | hours | each one, timed | name only | dropped |
| `coarse` | hours | how many, how long in total | not stored | dropped |

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

## The audit log

Everything that changes people, departments or agent tokens is recorded, along
with sign-ins and the attempts that failed:

```console
$ curl -H "Cookie: kasl_session=..." "http://127.0.0.1:8080/api/v1/audit?limit=2"
[{"id":4,"actor_id":null,"actor_email":"ivan@example.com","action":"auth.login_failed",
  "target_id":null,"target_label":null,"details":null,"at":"2026-08-22T00:58:12.880538Z"},
 {"id":3,"actor_id":"85440341-...","actor_email":"boss@example.com","action":"agent.issued",
  "target_id":"dcb60120-...","target_label":"ivan-laptop",
  "details":{"user_id":"9dce4dd0-..."},"at":"2026-08-22T00:58:11.850690Z"}]
```

Filter with `actor_id`, `target_id`, `action`, `since`, `until`, and page with
`limit` (500 at most) and `offset`. "Everything that happened to this person" is
`?target_id=...`.

**Nothing secret goes in.** An issued token is recorded as having been issued,
never as a value; a password change is recorded as having happened. A failed
sign-in keeps the address that was tried — a run of them against one account is
the thing worth seeing — but never the password, which is often a real one
belonging to somewhere else.

**There is no route to delete from it.** Not for old entries, not for a date
range. A journal the watched party can erase is not a journal, and the
administrator is the log'"'"'s main subject; trimming it is an operation for
whoever holds the database. Reading the log is not itself recorded — an audit of
the audit buries the actions under a log of people looking at the log.

Only an administrator may read it. The reasoning is in
[ADR 0010](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0010-the-audit-log.md).

## Importing history from before the server

Someone can track their time with kasl for a year before their team runs a
server. That history is an ordinary SQLite file on their machine, and it does
not have to be lost because the server arrived second:

```console
$ kasl-server import --db kasl.db --user employee@example.com --timezone -03:00
read 240 workdays, 312 pauses, 460 tasks from kasl.db
skipped 17 tasks the employee had deleted
imported 240 days as employee@example.com at -03:00
```

`--timezone` is required and has no default. kasl stores bare wall-clock text,
so nothing in the file says which offset it was recorded in - and a wrong guess
produces a perfectly plausible-looking year of work at the wrong hour. The
answer comes from whoever knows, and is echoed back so it is on the record.

- `--dry-run` reads and reports without writing anything.
- `--since` / `--until` bound the import by date, both ends inclusive. This is
  how someone who moved between time zones is imported correctly: one run per
  stretch, each with the offset that stretch was recorded in.
- The account must already exist - an import will not create it, so a typo in
  the email address cannot quietly file a year of history under a stranger.
- Re-importing replaces rather than duplicates, so a run that failed partway can
  simply be repeated, and a wrong offset is fixed by importing again with the
  right one.

The agent's file is opened read-only and never written to. Details and the
trade-offs behind the fixed offset are in
[ADR 0006](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0006-importing-local-history.md).

## The data model

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

## Installing it

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
      password: fvfcyap9bigg2qc8a3wj
```

Open `http://localhost:8080`, sign in with that, and change it. **The password
is printed once and stored nowhere else** — the alternative, writing one into a
file the server reads at every boot, leaves a credential lying around forever.
To name the administrator yourself instead, set `KASL_ADMIN=email:password`
before the first start.

The image is `ghcr.io/lacodda/kasl-server`, built for amd64 and arm64, so the
same compose file works on a laptop and on a Raspberry Pi. Pin a version in
production (`KASL_VERSION=0.14.0`); `latest` is for a first look.

Two more things before this holds a team's hours:

- **Put it behind HTTPS** and set `KASL_SECURE_COOKIES=true`. Over plain
  `http://` the session cookie cannot carry `Secure`, which is why the compose
  file ships with it off — convenient for a trial, wrong for anything real.
- **Take backups.** See below; the database's volume is the only copy until you
  do.

### Building from source instead

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

## Configuration

Everything comes from the environment:

| Variable | Meaning | Default |
| --- | --- | --- |
| `DATABASE_URL` | PostgreSQL connection string | required |
| `KASL_SERVER_ADDR` | Address the HTTP server binds to | `0.0.0.0:8080` |
| `KASL_AGENTS` | Agents to provision on startup, as `email:token` pairs separated by commas | none |
| `KASL_ADMIN` | First administrator, as `email:password`. Unset, the server generates one on a first run | none |
| `KASL_ADMIN_EMAIL` | Email for that generated administrator | `admin@kasl.local` |
| `KASL_SECURE_COOKIES` | Whether the session cookie carries `Secure`. Set `false` only when serving plain `http://` | `true` |
| `KASL_MAX_BATCH_DAYS` | Days one `/days/batch` request may carry | `31` |
| `KASL_MAX_BODY_BYTES` | Largest request body accepted | `4194304` |
| `RUST_LOG` | Log filter (tracing syntax) | `kasl_server=info,tower_http=info` |

Database migrations are embedded in the binary and applied on startup.

The privacy level is deliberately not here. It is set through the API and the
change is audited; an operator editing a file and restarting leaves nothing
behind that says who loosened the policy or when.

`KASL_AGENTS` is how the first agents get in while the admin UI does not exist
yet: each entry becomes an employee and an agent holding that token's hash.
Re-running with a changed token rotates it and revokes the old one. Tokens are
secrets — pass them through your deployment's secret store, not a committed
file — and the variable stops being the way in once tokens are issued from the
UI.

## What it will do

- Ingest work-time data from kasl agents: workdays, pauses, tasks, reports *(days, backfill and history import: done)*
- Manager dashboards: who is working right now, hours per person, trends over time *(the team's week and a drill-down into one person: done)*
- Personal pages: every employee sees their own history *(their own week, with the day timeline: done)*
- Roles: admin, manager, employee *(done)*
- Self-hosted: a single binary — API and web UI in one file — plus PostgreSQL, so your data stays on your infrastructure *(published image, install guide and backups: done)*

## Stack

Rust REST API (axum) + PostgreSQL (sqlx); the web UI is React 19 + TypeScript + Vite + Tailwind 4, built into the binary. Architectural decisions are recorded in [docs/adr/](https://github.com/lacodda/kasl-server/tree/main/docs/adr).

## License

[MIT](https://github.com/lacodda/kasl-server/blob/main/LICENSE)
