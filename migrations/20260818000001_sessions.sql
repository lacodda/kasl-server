-- Browser sessions, as opposed to the agents' long-lived tokens.
--
-- Kept server-side rather than signed into a self-contained token: an employee
-- who leaves must lose access at once, and "log out everywhere" has to mean it.
-- A stateless token cannot be withdrawn, and the blacklist that fixes that is
-- this table under another name (ADR 0007).
CREATE TABLE sessions (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Only the hash, for the same reason agent tokens store one: a database
    -- dump, or a stray log line, must not hand out a working session.
    token_hash text        NOT NULL UNIQUE,
    expires_at timestamptz NOT NULL,
    -- Rolling window: a session in daily use should not expire mid-afternoon,
    -- and one nobody has touched for weeks should.
    last_used_at timestamptz NOT NULL DEFAULT now(),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX sessions_user_id_idx ON sessions (user_id);
-- Expired rows are deleted on a schedule; the index keeps that sweep cheap.
CREATE INDEX sessions_expires_at_idx ON sessions (expires_at);
