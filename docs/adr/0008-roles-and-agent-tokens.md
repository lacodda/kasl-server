# 0008. What a manager may do, and how someone gets a password

Date: 2026-08-20

Status: Accepted

## Context

0.6.0 gave people a way to sign in and left them with almost nothing to do
afterwards. Adding someone still meant `KASL_AGENTS` on the host; stopping an
agent meant editing a row by hand. The `user_role` enum has existed since 0.2.0
(ADR 0003) and only ever gated one thing.

Two questions had to be settled before writing the routes.

**What may a manager do?** Departments arrive in 0.8.0. Until then a manager has
no group to be in charge of, so any authority they get is authority over the
whole company.

**How does a person end up with a password?** Accounts created by `KASL_AGENTS`
have none. The server has no mail channel and will not have one before 1.0, so
nothing can be sent to an address to prove it belongs to anyone.

## Decision

**A manager reads the team; an administrator changes it.** `GET /users` and
`GET /users/{id}/agents` admit both roles; everything else is admin-only.

Reading is granted now rather than withheld because a manager's dashboard
(0.13.0) is built on exactly that list, and because narrowing a permission later
breaks less than widening one: in 0.8.0 the same route starts returning only the
manager's own department, and no caller learns of a capability it is then
deprived of.

Issuing agent tokens was deliberately kept back. A token is the authority to
write someone's history, there is no audit log until 0.9.0 to notice who issued
what, and without departments the permission would not be "tokens for my team"
but "tokens for anyone here".

**An administrator sets an initial password; the person changes it.** `POST
/users` takes an optional password, and `POST /auth/password` takes the current
one and a new one. Verifying the current password is what stops a borrowed
unlocked laptop from becoming a permanent credential.

The alternative - a single-use invite link, so that no administrator ever knows
a colleague's password - is better in principle and does not survive contact
with this deployment. With no mail channel the link is handed over in person or
in a chat, exactly like a password, so the property it buys is mostly notional;
it costs a table, an expiry, a revocation path and an extra endpoint. When mail
exists, invites can replace this without changing anything a client already
depends on: `password` is optional, and an account without one simply cannot be
signed into.

**Passwords have a length floor and no other rule.** Eight characters. A server
that refuses `hunter2` while accepting `Passw0rd!` has chosen theatre over
arithmetic, and composition rules belong where they can be explained to the
person typing.

**Some consequences follow from a change instead of being asked for.**
Deactivating someone or resetting their password deletes their sessions - one is
someone leaving, the other is usually a suspicion, and in both cases leaving
open browsers behind defeats the point. Changing one's *own* password ends every
session but the one that made the change: being logged out of the browser you
just used is a poor reward for doing the right thing.

**The last administrator cannot be demoted or deactivated.** Either would leave
an installation nobody can administer, recoverable only by running the `admin`
subcommand on the host - which is not where someone who just clicked a toggle in
a browser will think to look.

**An agent token is shown once**, prefixed `kasl_` so it is recognisable in a
log or a config file, and stored as a SHA-256 like the seeded ones (ADR 0004).
Revoking is idempotent but does not move `revoked_at`: when access ended is a
fact, and a second click must not rewrite it.

## Consequences

- `KASL_AGENTS` and `KASL_ADMIN` still work and still make sense: they are how a
  container gets its first account, before anyone can sign in to create one.
- An employee cannot list the team. Who else works somewhere is not an
  employee's to enumerate, and the refusal says only "not allowed".
- There is no delete for a user, only deactivation. Their days must keep an
  owner; a row with `active = false` is what a departed colleague looks like.
- Reactivating someone does not restore their sessions - the rows were deleted,
  not flagged - and does not restore a revoked agent. Both are re-issued
  deliberately, which is the intent.
- Nothing here is recorded beyond a log line. Who created whom and who issued
  which token becomes queryable with the audit log in 0.9.0.
