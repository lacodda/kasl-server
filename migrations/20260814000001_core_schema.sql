-- Core schema: the entities kasl agents report and the web UI reads back.
--
-- Shape follows the agent's local model (workdays, pauses, tasks, tags) so an
-- upload maps one to one, with the differences a multi-user server forces:
--
--   * Every row belongs to a user. The agent has no such column: its database
--     holds one person.
--   * Instants are `timestamptz`. The agent stores wall-clock text with no
--     offset, which is unambiguous on one machine and meaningless across a
--     team in several time zones. Agents send an offset; the server keeps the
--     absolute instant plus the local calendar date the agent assigned it, so
--     "Tuesday" stays the employee's Tuesday no matter where the server runs.
--   * Days, pauses and tasks are linked by foreign key. The agent relates them
--     by `date(start)` string comparison; here the day owns its rows.

-- Every table below carries an owner and timestamps, so both are declared once.
CREATE FUNCTION set_updated_at() RETURNS trigger AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- People ---------------------------------------------------------------------

-- Roles are an enum rather than free text: the set is small, closed, and every
-- authorization check compares against it.
CREATE TYPE user_role AS ENUM ('admin', 'manager', 'employee');

CREATE TABLE users (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Login handle. Case-insensitive uniqueness is enforced by the index
    -- below; the original spelling is preserved for display.
    email         text        NOT NULL,
    display_name  text        NOT NULL,
    role          user_role   NOT NULL DEFAULT 'employee',
    -- Password verifier. Null while an account exists but has no way in yet
    -- (invited, or authenticated through a directory later on).
    password_hash text,
    -- Deactivation instead of deletion: a departed employee's history must
    -- survive, and their agent must stop being accepted.
    active        bool        NOT NULL DEFAULT true,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX users_email_key ON users (lower(email));

CREATE TRIGGER users_set_updated_at BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Agents ---------------------------------------------------------------------

-- One installed kasl. A person may run several (desktop and laptop), so the
-- agent is a separate identity that reports on the user's behalf.
CREATE TABLE agents (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Human label in the admin UI, e.g. the machine name the agent reports.
    name        text        NOT NULL,
    -- Only the hash of the bearer token is stored: a database dump must not
    -- hand out working credentials. The token itself is shown once, at issue.
    token_hash  text        NOT NULL UNIQUE,
    -- Set when the token is withdrawn; the row stays so its data keeps an
    -- owner and the audit trail stays intact.
    revoked_at  timestamptz,
    -- Last accepted request, for the "agent went silent" signals.
    last_seen_at timestamptz,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX agents_user_id_idx ON agents (user_id);

CREATE TRIGGER agents_set_updated_at BEFORE UPDATE ON agents
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Workdays -------------------------------------------------------------------

CREATE TABLE workdays (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- The employee's local calendar date, exactly as the agent recorded it.
    -- Not derived from `started_at`: near midnight, or after travel, the
    -- server's idea of the date is not the employee's.
    date        date        NOT NULL,
    started_at  timestamptz NOT NULL,
    -- Null while the day is still open, as in the agent.
    ended_at    timestamptz,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT workdays_end_after_start CHECK (ended_at IS NULL OR ended_at >= started_at)
);

-- One row per person per day, matching the agent's UNIQUE(date). This is also
-- what makes a re-upload an upsert rather than a duplicate.
CREATE UNIQUE INDEX workdays_user_date_key ON workdays (user_id, date);

CREATE TRIGGER workdays_set_updated_at BEFORE UPDATE ON workdays
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Pauses ---------------------------------------------------------------------

CREATE TABLE pauses (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    workday_id uuid        NOT NULL REFERENCES workdays (id) ON DELETE CASCADE,
    started_at timestamptz NOT NULL,
    -- Null while the pause is still running on the agent.
    ended_at   timestamptz,
    -- Seconds, the unit the agent uses. Stored rather than computed because a
    -- merged pause is not always `ended_at - started_at`: the agent joins
    -- neighbours across a configurable gap before uploading.
    duration_seconds integer,
    -- Manual break entered by the employee (the agent's `protected` flag):
    -- exempt from the short-pause thresholds and never merged away.
    manual     bool        NOT NULL DEFAULT false,
    -- Free-text explanation for a manual break.
    reason     text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pauses_end_after_start CHECK (ended_at IS NULL OR ended_at >= started_at),
    CONSTRAINT pauses_duration_not_negative CHECK (duration_seconds IS NULL OR duration_seconds >= 0)
);

CREATE INDEX pauses_workday_id_idx ON pauses (workday_id, started_at);

CREATE TRIGGER pauses_set_updated_at BEFORE UPDATE ON pauses
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Tasks ----------------------------------------------------------------------

CREATE TABLE tasks (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- The agent's own row id, unique per user. It is how a re-upload finds the
    -- row it already sent instead of creating a second one.
    agent_task_id integer  NOT NULL,
    -- The agent's `task_id`: the key that ties the same piece of work carried
    -- across several days. Equal to `agent_task_id` for a task started today.
    agent_group_id integer NOT NULL,
    -- The employee's local date the task was recorded on. The agent relates
    -- tasks to days by `date(timestamp)`; carrying the date keeps that grouping
    -- exact instead of re-deriving it in another time zone.
    date       date        NOT NULL,
    recorded_at timestamptz NOT NULL,
    name       text        NOT NULL,
    comment    text,
    -- Percent complete, 0..100.
    completeness smallint  NOT NULL DEFAULT 100,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT tasks_completeness_range CHECK (completeness BETWEEN 0 AND 100)
);

CREATE UNIQUE INDEX tasks_user_agent_task_key ON tasks (user_id, agent_task_id);
CREATE INDEX tasks_user_date_idx ON tasks (user_id, date);
CREATE INDEX tasks_user_group_idx ON tasks (user_id, agent_group_id);

CREATE TRIGGER tasks_set_updated_at BEFORE UPDATE ON tasks
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Tags are per user: two employees may both have "review" and they are not the
-- same tag, nor should one see the other's vocabulary.
CREATE TABLE tags (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id    uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name       text        NOT NULL,
    -- Display color as the agent stores it; no format is imposed here.
    color      text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX tags_user_name_key ON tags (user_id, name);

CREATE TRIGGER tags_set_updated_at BEFORE UPDATE ON tags
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE task_tags (
    task_id uuid NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
    tag_id  uuid NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    PRIMARY KEY (task_id, tag_id)
);

CREATE INDEX task_tags_tag_id_idx ON task_tags (tag_id);

-- Reports --------------------------------------------------------------------

-- A report is an event, not a second copy of the day: the agent computes its
-- figures on the fly from workdays, pauses and tasks, and so does this server.
-- What cannot be recomputed later is that a report was submitted, when, and
-- with which numbers at that moment - which is what approval and the "late /
-- missing report" signals are built on.
CREATE TYPE report_kind AS ENUM ('daily', 'monthly');

CREATE TABLE reports (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind        report_kind NOT NULL,
    -- First day of the period the report covers: the day itself for a daily
    -- report, the first of the month for a monthly one.
    period_start date       NOT NULL,
    submitted_at timestamptz NOT NULL,
    -- Figures as of submission, kept so an approved report does not change
    -- under the approver when later data arrives.
    worked_seconds integer  NOT NULL DEFAULT 0,
    -- Percent, as the agent reports productivity. `real` rather than
    -- `numeric`: this is a measurement shown to one decimal, never money and
    -- never summed, so exact decimal arithmetic buys nothing.
    productivity real,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT reports_worked_seconds_not_negative CHECK (worked_seconds >= 0),
    CONSTRAINT reports_productivity_range CHECK (productivity IS NULL OR productivity BETWEEN 0 AND 100)
);

CREATE UNIQUE INDEX reports_user_period_key ON reports (user_id, kind, period_start);

CREATE TRIGGER reports_set_updated_at BEFORE UPDATE ON reports
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
