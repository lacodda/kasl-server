-- Webhooks: what this server said to the outside, and whether it got there.
--
-- Until now every fact the server held waited to be opened. Alerts made it
-- notice things on its own (ADR 0018); this is where it starts saying them to
-- somewhere a manager already is - a team chat, or a system of their own.
--
-- **Where they go is not in this schema.** A Slack webhook URL and a Telegram
-- bot token are working credentials to post as somebody, and this database is
-- what `kasl-server backup` writes to a file: the destinations are declared in
-- the deployment's environment (`KASL_WEBHOOK_<NAME>`), next to the database
-- password, and a row here names a destination only by that label (ADR 0019).
--
-- **A row is one event for one destination.** Written in the same transaction
-- as the fact it announces - the alert raised, the day accepted - so an event
-- cannot be lost between "it happened" and "it was queued", which is the
-- failure an in-memory channel has on every restart. The dispatcher then
-- sends what is due, retries what failed, and gives up in writing.

-- What happened --------------------------------------------------------------
--
-- A closed set, like the alert rules: a receiver branches on it, and free text
-- would make a typo an event nothing renders.
--
--   * `alert.raised` / `alert.acknowledged` / `alert.resolved` - the life of an
--     alert, told as it moves. Resolution is sent too: "the agent is back" is
--     the message that lets a channel stop worrying, and without it every
--     alert posted is a question nobody closes.
--   * `day.closed` - a day arrived finished for the first time.
--   * `test` - an administrator asked for one, to see the channel works.
CREATE TYPE webhook_event AS ENUM ('alert.raised', 'alert.acknowledged', 'alert.resolved', 'day.closed', 'test');

CREATE TABLE webhook_deliveries (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Shared by every destination the one event went to. What a receiver
    -- deduplicates on: delivery is at least once, because a request that
    -- timed out may still have arrived.
    event_id        uuid          NOT NULL,
    -- The label from `KASL_WEBHOOK_<NAME>`, lower-cased. Never the address.
    destination     text          NOT NULL,
    event           webhook_event NOT NULL,
    -- Who it is about. Cascades with the account, like their alerts and their
    -- days; null for a test.
    user_id         uuid REFERENCES users (id) ON DELETE CASCADE,
    -- The event as it was true when it happened, in the shape the `json`
    -- destinations receive. Chat messages are rendered from it at send time;
    -- the figures are fixed here, for the reason an alert's are (ADR 0018).
    payload         jsonb         NOT NULL,
    created_at      timestamptz   NOT NULL DEFAULT now(),
    attempts        int           NOT NULL DEFAULT 0,
    -- When the dispatcher may try next. Moves forward on each failure.
    next_attempt_at timestamptz   NOT NULL DEFAULT now(),
    delivered_at    timestamptz,
    -- Given up on: the receiver refused it for good, or it kept failing past
    -- the last retry. Kept rather than deleted - "we stopped trying at 03:10
    -- because Slack said the hook was gone" is the answer an operator needs.
    abandoned_at    timestamptz,
    -- The last HTTP status the receiver answered, when it answered at all.
    last_status     int,
    -- Why the last attempt failed, in words. Never carries the address: the
    -- dispatcher strips it before this is written.
    last_error      text,
    CONSTRAINT webhook_deliveries_one_outcome CHECK (delivered_at IS NULL OR abandoned_at IS NULL),
    CONSTRAINT webhook_deliveries_attempts_not_negative CHECK (attempts >= 0),
    -- One row per event per destination, so a sweep that races itself cannot
    -- post the same alert twice.
    CONSTRAINT webhook_deliveries_event_destination_key UNIQUE (event_id, destination)
);

-- What the dispatcher reads every few seconds: per destination, the oldest
-- thing still in flight. Partial, so a year of delivered rows costs nothing.
CREATE INDEX webhook_deliveries_pending_idx ON webhook_deliveries (destination, created_at)
    WHERE delivered_at IS NULL AND abandoned_at IS NULL;

-- The history screen: newest first.
CREATE INDEX webhook_deliveries_created_idx ON webhook_deliveries (created_at DESC);
