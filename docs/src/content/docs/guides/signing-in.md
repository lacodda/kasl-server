---
title: Signing in
description: Email-and-password sessions for people, alongside the bearer tokens kasl agents keep using.
---

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
