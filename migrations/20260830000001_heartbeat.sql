-- The agent's pulse: what a machine says its person is doing right now.
--
-- Until now the closest thing to a live status was `agents.last_seen_at`, a
-- stamp written whenever a request arrived. That answers "when did we last
-- hear from this machine" and nothing else: an agent that uploads once at
-- midnight looks identical to one whose employee is at the keyboard. The
-- dashboard said so out loud rather than guess.
--
-- The pulse is the agent's own claim, sent on its own schedule, and it is
-- deliberately kept apart from `last_seen_at`: one is "the token was used",
-- the other is "kasl is watching a person work". A server that conflated them
-- would report a backfill of last month as somebody working tonight.

-- The states an agent may claim. Closed set, like every other enum here: the
-- dashboard branches on it and free text would make a typo a fourth state.
--
-- `idle` rather than `stopped`: the agent is running and reporting, the person
-- is simply not in a working day. An agent that has stopped sends nothing at
-- all, which is the absence of a pulse, not a value of one.
CREATE TYPE agent_state AS ENUM ('working', 'paused', 'idle');

ALTER TABLE agents
    -- The last state claimed. Null on an agent that has never sent a pulse -
    -- every agent shipped before this column, and every kasl too old to know
    -- the endpoint. Null must therefore read as "unknown", never as "idle".
    ADD COLUMN heartbeat_state agent_state,
    -- When the agent says it observed that state, with its own clock.
    ADD COLUMN heartbeat_at timestamptz,
    -- When the server received it. Kept alongside the agent's own stamp for
    -- the same reason ingest keeps both (ADR 0003): staleness is measured
    -- against the server's clock, because a machine whose clock is a day
    -- behind would otherwise look permanently offline - or, worse, a machine
    -- whose clock runs fast would look alive forever.
    ADD COLUMN heartbeat_received_at timestamptz;

-- The dashboard asks for the freshest pulse per person on every poll.
CREATE INDEX agents_heartbeat_received_at_idx ON agents (heartbeat_received_at DESC NULLS LAST);

-- How old the demo keeps this agent's pulse, in seconds.
--
-- Only the demo writes it. The fictional team has to show every state the
-- dashboard can render, including "this machine stopped answering", and that
-- row has to survive the periodic re-stamp that keeps the live ones live.
-- Deriving it - treating any already-stale pulse as deliberate - cannot work:
-- a pulse that merely aged looks identical to one seeded old, so the intent
-- is recorded rather than guessed.
--
-- Null on every real agent, and nothing outside `demo` reads it.
ALTER TABLE agents ADD COLUMN demo_pulse_age_seconds int;
