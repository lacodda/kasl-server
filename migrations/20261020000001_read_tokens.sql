-- Read tokens: how a BI tool reads the team's hours without a person signed in
-- (ADR 0025).
--
-- A table of their own rather than a kind of agent. An agent token writes one
-- person's days; a read token reads everybody its owner may see. Kept in one
-- table, the two would be one missing `WHERE kind = ...` away from a dashboard
-- that can file days under somebody's name.

-- What a token may read ---------------------------------------------------------
--
-- One value today. The finance milestone adds `payroll`, so that the money can
-- go to a dashboard without the hours going with it - and a scope that exists
-- from the first token is a check every BI route already makes, rather than one
-- added later to routes written without it.
CREATE TYPE read_scope AS ENUM ('hours');

CREATE TABLE read_tokens (
    id           uuid        PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Whose eyes the token reads with. Not who may see the token: the owner's
    -- role and department are read at every request, so a token narrows when
    -- its owner's authority does and stops when their account does.
    --
    -- CASCADE like agents: an account is deactivated, never deleted, in a
    -- running installation; only the demo's reseed deletes people, and then
    -- their tokens go with them.
    owner_id     uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- What it is for, as the owner named it: "Grafana", "the finance sheet".
    name         text        NOT NULL CHECK (btrim(name) <> ''),
    scope        read_scope  NOT NULL,
    -- SHA-256 of the token, like an agent's: a long random string the server
    -- issued has no dictionary to slow anybody down with (ADR 0004).
    token_hash   text        NOT NULL UNIQUE,
    created_at   timestamptz NOT NULL DEFAULT now(),
    -- The last request it answered. A dashboard polls, so this moves; a token
    -- that has not moved in months is one to revoke.
    last_used_at timestamptz,
    -- When access ended. The row stays: who could read what, and until when,
    -- is history the audit log points at.
    revoked_at   timestamptz
);

-- "My tokens" for a manager, every token for an administrator.
CREATE INDEX read_tokens_owner_idx ON read_tokens (owner_id);
