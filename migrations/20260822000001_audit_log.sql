-- The audit log: who did what, to whom, and when.
--
-- Roles without a trace do not earn trust (ADR 0010). The tracing output
-- already carries most of this, but a log file cannot answer "who looked at
-- this person's pay last quarter" - that needs rows.
CREATE TABLE audit_log (
    id         bigserial PRIMARY KEY,
    -- Who acted. Null for something the server did on its own - provisioning
    -- from the environment at startup has no person behind it.
    --
    -- ON DELETE SET NULL: users are deactivated rather than deleted, but if a
    -- row ever does go, the record of what they did must not go with it. That
    -- is the whole point of an audit log.
    actor_id   uuid REFERENCES users (id) ON DELETE SET NULL,
    -- Kept as text alongside the id: an account can be renamed or its row can
    -- vanish, and an entry that reads "someone changed a role" helps nobody.
    actor_email text,
    -- Dotted verb: `user.created`, `agent.revoked`, `auth.login`. Text rather
    -- than an enum - a new action must not need a migration, and the set is
    -- open by design.
    action     text        NOT NULL,
    -- What was acted upon, when there is one thing.
    target_id  uuid,
    target_label text,
    -- Whatever else is worth keeping, per action. Never credentials: this
    -- table is read by an admin UI and copied into support tickets.
    details    jsonb,
    -- Where the request came from, when the deployment can tell. Behind a
    -- reverse proxy this is whatever the proxy passes on.
    ip         text,
    at         timestamptz NOT NULL DEFAULT now()
);

-- The two questions actually asked of this table: "what happened lately" and
-- "everything about this person".
CREATE INDEX audit_log_at_idx ON audit_log (at DESC);
CREATE INDEX audit_log_actor_idx ON audit_log (actor_id, at DESC);
CREATE INDEX audit_log_target_idx ON audit_log (target_id, at DESC);
CREATE INDEX audit_log_action_idx ON audit_log (action, at DESC);
