---
title: Managing the team
description: Once an administrator exists, people, departments and agent tokens are managed over the API.
---

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
