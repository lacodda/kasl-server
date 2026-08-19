# 0007. Sessions in a table, and where the first administrator comes from

Date: 2026-08-18
Status: Accepted

## Context

Until now every route was one of two things: open, or authenticated as a kasl
agent holding a bearer token. The schema has had `users.password_hash` and a
`user_role` enum since 0.2.0 (ADR 0003) and nothing used them. The web UI
arrives in 0.11.0 and needs someone to be signed in before it has anything to
show.

Two questions had to be answered first.

**What carries a session?** A signed self-contained token (JWT) that the server
does not store, or a random token whose hash sits in a table.

**Where does the first administrator come from?** Accounts today are created
only as a side effect of `KASL_AGENTS`, and every one of them has
`password_hash = NULL` - nobody can sign in at all.

## Decision

**Sessions are rows.** Logging in generates 32 random bytes, hands them to the
browser in an `HttpOnly; SameSite=Strict` cookie, and stores their SHA-256 in
`sessions`. Every request resolves the cookie against that table.

The cost is a query per request, which for a team of dozens is nothing worth
optimising. What it buys is the ability to end access: an employee who leaves
loses their sessions the afternoon they leave, and "log out everywhere" is a
single `DELETE`. A stateless token cannot be withdrawn before it expires, and
the standard fix - a revocation list checked on each request - is this table
under a different name, with the drawbacks of both designs. A short-lived access
token plus a stored refresh token was rejected for the same reason at twice the
code: it still leaves a window in which a dismissed employee is authenticated,
and the window is exactly the thing being bought with the complexity.

`SameSite=Strict` is what makes CSRF tokens unnecessary here: every caller is
our own page on our own origin, and there is no third-party context to protect.
`Secure` is on by default and can be turned off with `KASL_SECURE_COOKIES=false`
- a `Secure` cookie over plain `http://` is silently dropped by the browser,
which presents as "login does nothing at all", and a stand on a home network is
a real deployment.

Expiry is a rolling fortnight: each authenticated request pushes it out again,
so a session in daily use does not die mid-afternoon while a forgotten one does.

**Passwords get Argon2id**, unlike the agents' tokens which keep SHA-256. The
distinction is not inconsistency: a token is a long random string the server
issued and there is no dictionary to slow an attacker down with, whereas a
password is something a person chose and is guessable at scale.

**An unknown email, a deactivated account and a wrong password are one answer.**
Distinguishing them turns the login form into a way to enumerate who works at a
company. The unknown-email path verifies the supplied password against a fixed
dummy hash before refusing, so the two do not differ by a measurable pause
either.

**The first administrator is made deliberately**, by `kasl-server admin --email
… --password …` or `KASL_ADMIN=email:password` in the environment - the same
shape as `KASL_AGENTS`, because a container has no other way to be handed a
first account. The command upserts: it creates the account, or resets the
password and promotes an existing one. Both matter. An operator who locked
themselves out has no other way back in, and the realistic first admin is
someone whose account already exists because their agent has been reporting for
weeks - so it has to be the same row, or their history would be orphaned.

"First person to sign in becomes the administrator" was rejected: between
starting the server and that first login, anyone who can reach it is the
administrator.

## Consequences

- `require_admin` exists on the current user, and roles otherwise wait for
  0.7.0. It is here now because the account-management routes arrive with that
  milestone and would otherwise be written twice.
- Accounts created by `KASL_AGENTS` still cannot be signed into. That is
  correct: they exist to own data an agent uploads, and giving them a way in
  they never asked for is a door nobody opened deliberately.
- Deactivating a user stops their sessions from authenticating, but does not
  delete the rows. `revoke_all` exists for when the account-management UI wants
  to do both.
- Expired sessions are refused by the query and deleted by a sweep at startup.
  A server that runs for months without restarting accumulates dead rows; when
  that matters, the sweep gets a timer rather than a new mechanism.
- There is no password reset for anyone but an administrator, and no way for an
  employee to change their own. Both belong with the account-management UI.
