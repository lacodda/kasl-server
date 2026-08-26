-- The privacy policy: how much detail this installation stores about a person.
--
-- Enforced at ingest, not at read time (ADR 0011). What a level excludes is
-- dropped before the day is written, so the promise is about the disk rather
-- than about the user interface.
CREATE TYPE privacy_level AS ENUM ('full', 'moderate', 'coarse');

-- Installation-wide settings. One row, forever: `singleton` is the constraint
-- that says so, rather than a comment asking the next reader to be careful.
--
-- A table rather than an environment variable, because the level is changed by
-- an administrator through the API and the change is audited. An operator
-- editing a `.env` and restarting leaves nothing behind that says who loosened
-- the policy or when.
CREATE TABLE settings (
    singleton     bool PRIMARY KEY DEFAULT true CHECK (singleton),
    -- Defaults to `full`: what every version up to this one did. A timid
    -- default would silently start discarding data in installations that are
    -- already running, and the loss is permanent (ADR 0011).
    privacy_level privacy_level NOT NULL DEFAULT 'full',
    updated_at    timestamptz   NOT NULL DEFAULT now()
);

CREATE TRIGGER settings_set_updated_at BEFORE UPDATE ON settings
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- The row exists from the start, so every reader can assume it is there and
-- none of them has to carry a "what if the settings are missing" branch.
INSERT INTO settings (singleton) VALUES (true);

-- What `coarse` keeps of a day's pauses.
--
-- Under that level individual pauses are not stored at all, so without these
-- the day would claim uninterrupted work - a more flattering picture than the
-- truth, and a false one. The count and the total say the same thing about the
-- day's hours as the rows would, without saying when the person stepped away.
--
-- Null under the other levels, where the pauses themselves answer this.
ALTER TABLE workdays ADD COLUMN paused_count integer
    CHECK (paused_count IS NULL OR paused_count >= 0);
ALTER TABLE workdays ADD COLUMN paused_seconds integer
    CHECK (paused_seconds IS NULL OR paused_seconds >= 0);
