-- Notifications: what the server told one person, and how far they have read.
--
-- Everything the server said before this, it said to a manager: alerts wait on
-- the dashboard (ADR 0018) and go to a team's chat (ADR 0019). The person an
-- alert is about heard nothing - including about the one alert they can fix
-- themselves, a day the server has held open for seventeen hours because the
-- close kasl made on the laptop never arrived.
--
-- **A row is one message to one person.** Stored, like an alert and for the
-- same reason: when it was said and whether it was read are facts no other
-- row carries, and a notice computed on read could never reach a machine that
-- was asleep when it happened. Written in the transaction that made its fact
-- true, so the fact and the telling commit together (ADR 0020).

-- What it is about ------------------------------------------------------------
--
-- A closed set, like the alert rules and the webhook events: the agent and the
-- inbox branch on it, and free text would make a typo a kind nothing renders.
-- Each is a fact the employee cannot see from where they are:
--
--   * `alert.raised`    - the server raised an alert about you; your manager
--                         was told the same sentence.
--   * `agent.issued`    - a new token can report as you. The notice every
--                         account system sends for a new sign-in: if it was not
--                         you, you are the only one who can say so.
--   * `agent.revoked`   - a machine can no longer report as you.
--   * `privacy.changed` - what this server keeps about your days changed
--                         (ADR 0011).
--
-- Later milestones add values here - a manager's note on a day, a day
-- approved, a report that did not arrive - rather than channels of their own.
CREATE TYPE notification_kind AS ENUM ('alert.raised', 'agent.issued', 'agent.revoked', 'privacy.changed');

CREATE TABLE notifications (
    -- A number and not a uuid: it is the order a person reads in and the
    -- cursor an agent is told to acknowledge up to. A uuid has no order, and
    -- sorting by `created_at` ties inside one transaction - a privacy change
    -- writes a row for everybody in the same instant.
    id          bigserial PRIMARY KEY,
    -- Who it was said to. Cascades: an account removed from the installation
    -- takes what it was told with it, as it takes its days.
    user_id     uuid              NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind        notification_kind NOT NULL,
    -- The alert it announces. Unique: one alert is told once, even when two
    -- sweeps race to raise it. Whether the notice is still true is read from
    -- the alert's own `resolved_at` rather than copied here, so the two cannot
    -- disagree.
    alert_id    uuid UNIQUE REFERENCES alerts (id) ON DELETE CASCADE,
    -- The machine it is about, for the two kinds that are about one.
    agent_id    uuid REFERENCES agents (id) ON DELETE CASCADE,
    -- The facts as they were when the person was told - the figures of the
    -- alert, the machine's name, the two levels. Never re-derived: the
    -- sentence somebody read is the one that was true when they read it
    -- (ADR 0018). The words are rendered from this at read time.
    payload     jsonb             NOT NULL,
    created_at  timestamptz       NOT NULL DEFAULT now(),

    -- A reference exists exactly when the kind has one. An `alert.raised` with
    -- no alert could never be withdrawn; an `agent.issued` pointing nowhere
    -- would name a machine nobody can find.
    CONSTRAINT notifications_alert_matches_kind CHECK ((kind = 'alert.raised') = (alert_id IS NOT NULL)),
    CONSTRAINT notifications_agent_matches_kind CHECK ((kind IN ('agent.issued', 'agent.revoked')) = (agent_id IS NOT NULL))
);

-- A person's inbox, newest first, and an agent's queue, oldest first after its
-- cursor. One index serves both.
CREATE INDEX notifications_user_id_id_idx ON notifications (user_id, id);

-- How far along each reader is --------------------------------------------------
--
-- Cursors and not flags on rows: both only ever move forward, and a list read
-- top to bottom is read *up to* a point. Two of them, because they are two
-- facts (ADR 0020):
--
--   * shown on a machine - per agent. A second machine gets its own toast
--     rather than none: a notice missed on the desk nobody is at is worse than
--     the same notice twice.
--   * seen by the person - per person. The web inbox opened, or kasl saying
--     the person clicked. Something seen is not toasted anywhere afterwards.
--
-- Zero is "nothing yet", and ids start at one.
ALTER TABLE agents ADD COLUMN notified_through bigint NOT NULL DEFAULT 0 CHECK (notified_through >= 0);
ALTER TABLE users ADD COLUMN notifications_read_through bigint NOT NULL DEFAULT 0 CHECK (notifications_read_through >= 0);
