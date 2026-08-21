# 0009. Departments, and who is visible to nobody

Date: 2026-08-21

Status: Accepted

## Context

0.7.0 gave a manager the right to read the team and noted the debt out loud:
with no group to be in charge of, "manager" meant "may read the whole company"
(ADR 0008). That is tolerable while a company fits on one screen and stops being
so at about the point the product becomes worth buying.

Two questions had to be settled before the schema.

**How is a manager tied to a group?** Either the group names its head, or a
manager is recognised by being a member of it, or the relation is many-to-many.

**Who sees the people nobody has filed yet?** There will always be some: a
new hire, the administrator themselves, and every account that exists on the day
this ships.

## Decision

**A department names its manager; a person belongs to one department.**
`departments.manager_id` and `users.department_id`. A manager sees the people in
the departments they run, plus themselves.

Recognising a manager by membership was rejected for what it cannot express: two
managers in one group see each other with no way to say which one runs it, and
"this department is currently between heads" - a real state during a handover -
becomes unrepresentable. A many-to-many table is the honest model for a manager
who runs several groups, and costs a table and a join in every visibility query
for a case nobody has yet; `manager_id` can grow into it later without any
client noticing, because the API speaks in departments rather than in the shape
of the join.

Themselves is included deliberately: a manager who runs nothing yet would
otherwise get an empty page and report the product as broken.

**Someone with no department is visible to the administrator alone.**

The alternative - showing the unfiled to every manager, so that nobody falls
through - fails in the direction that cannot be observed. Forget to file a
person and they are exposed to every manager in the company, silently, and
nothing in any screen says so. Under this rule the same mistake makes them
missing from a list, which is reported the same afternoon. Where a default has
to be wrong, it should be wrong loudly.

This also decides the migration: the column is nullable and everyone starts
unfiled, rather than being swept into a "General" department that would put the
administrator and the service accounts somewhere they do not belong.

**Deleting a department does not delete its people.** `ON DELETE SET NULL` on
both sides: its members become unfiled, its manager reference clears. A button
that removes a group must never be a button that removes the people in it.

**An employee cannot be made to run a department.** They could not see it -
`GET /users` admits managers and admins only - so the department would silently
have no working head. The refusal says what to do instead.

## Consequences

- **This is a breaking change to `GET /api/v1/users`** for managers: it returns
  fewer people than it did in 0.7.0. Administrators and agents are unaffected.
  It is the narrowing that ADR 0008 said would come, and it arrives before any
  client exists to be broken by it.
- Departments themselves are listed to any manager, not just their own. Knowing
  the company has a "Sales" is not the disclosure worth a second query path -
  the people inside it are what the user list scopes.
- A manager can see a person in their department but cannot change them.
  Administration stayed with the administrator (ADR 0008), and departments give
  a scope to read, not a licence to reorganise.
- Nothing prevents a manager from being filed into a department run by someone
  else. That is a real arrangement - a team lead reporting to a head of
  department - and it means each simply sees their own scope.
- One department per person. Someone splitting time between two teams is
  currently filed under one of them; if that turns out to matter, membership
  becomes its own table without changing what the API says.
