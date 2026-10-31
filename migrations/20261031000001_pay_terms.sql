-- Pay terms: how a person is paid, version by version (ADR 0027).
--
-- The first table of the finance milestone. Money earned is derived from the
-- hours the agents send and the terms in force on each date; this table holds
-- the terms, and every version of them that was ever recorded.

-- What a rate is a rate of. Piecework and one-off bonuses are layers on top of
-- a basis, not bases of their own, so "mixed" pay is not a value here.
CREATE TYPE pay_basis AS ENUM ('hourly', 'daily', 'monthly');

CREATE TABLE pay_terms (
    -- Also the order in which versions were recorded: of two versions starting
    -- on the same date, the one with the larger id is in force. A counter
    -- rather than a timestamp, because two writes can share a clock reading
    -- and a tie would leave the date with two rates.
    id             bigserial      PRIMARY KEY,
    -- CASCADE like the person's days: an account is deactivated, never
    -- deleted, in a running installation; only the demo's reseed deletes
    -- people, and then their terms go with them.
    user_id        uuid           NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- The first date these terms apply to, in the person's own calendar. There
    -- is no end date: a version ends where the next one begins, so two
    -- versions can neither overlap nor leave a gap between them.
    effective_from date           NOT NULL,
    basis          pay_basis      NOT NULL,
    -- Per hour worked, per day worked or per month, in `currency`. Exact
    -- decimal: money is never a float here. The server refuses a fifth decimal
    -- place before the insert, so the column never rounds one away unseen.
    rate           numeric(18, 4) NOT NULL CHECK (rate > 0),
    -- ISO 4217. The server checks the code against the standard's list; this
    -- only keeps the shape.
    currency       text           NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    -- Why the terms changed: "annual review", "moved to a salary". Read by the
    -- person the terms are about as well as by whoever recorded them.
    note           text           CHECK (note IS NULL OR (btrim(note) <> '' AND char_length(note) <= 500)),
    -- Who entered it. Never set to null and never cascaded: deleting the
    -- person who recorded somebody else's terms is refused, because the
    -- history would lose its author. Checked at commit rather than per row,
    -- for the one statement that deletes accounts - the demo's reseed removes
    -- everybody at once, and row by row an administrator would still be
    -- named by terms whose own account had not been reached yet. The column
    -- is written once, from the session, so deferring its check costs nothing.
    recorded_by    uuid           NOT NULL REFERENCES users (id) DEFERRABLE INITIALLY DEFERRED,
    recorded_at    timestamptz    NOT NULL DEFAULT now()
);

-- A person's history, in the order the rule "in force on a date" reads it.
CREATE INDEX pay_terms_user_idx ON pay_terms (user_id, effective_from, id);

-- A version is history. Changing a rate is recording a new version, and
-- correcting a mistake is recording one with the same start; neither rewrites
-- what money already computed may have used. A row goes only with its account:
-- the cascade from `users` runs inside the foreign key's own trigger, one level
-- deeper than a statement aimed at this table.
CREATE FUNCTION pay_terms_are_history() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' AND pg_trigger_depth() > 1 THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'pay terms are history: record a new version instead of changing or removing one'
        USING ERRCODE = 'restrict_violation';
END;
$$;

CREATE TRIGGER pay_terms_are_history BEFORE UPDATE OR DELETE ON pay_terms
    FOR EACH ROW EXECUTE FUNCTION pay_terms_are_history();
