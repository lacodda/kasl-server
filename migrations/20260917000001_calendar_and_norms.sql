-- The production calendar and the norm: the first standard this server holds.
--
-- Everything up to here compared a person with themselves (ADR 0016) because
-- there was nothing else to compare them with. A norm is that something else,
-- and it arrives as three separate facts rather than one number, because one
-- number is wrong in three different ways.

-- What makes a date unlike the weekday it falls on ------------------------

-- Only exceptions are stored. A year is 365 rows if every day is listed and
-- about a dozen if only the surprises are, and the dozen is also the set an
-- administrator can check against the decree it came from.
--
-- Three kinds, not one, because a production calendar does three things:
--
--   * `holiday` - a working weekday that is not worked.
--   * `short_day` - the eve of a holiday, one hour shorter. Without it the
--     calendar would owe an hour that nobody worked, every time.
--   * `working_weekend` - a Saturday moved into the working week when a
--     holiday is transferred. This is why a flat list of holidays cannot
--     express a calendar: it can only ever subtract.
CREATE TYPE calendar_day_kind AS ENUM ('holiday', 'short_day', 'working_weekend');

CREATE TABLE calendar_days (
    date       date              PRIMARY KEY,
    kind       calendar_day_kind NOT NULL,
    -- What the day is called, for the screen that lists the year. Optional:
    -- the date and the kind are the calendar, the name is for people.
    note       text,
    created_at timestamptz       NOT NULL DEFAULT now(),
    updated_at timestamptz       NOT NULL DEFAULT now()
);

CREATE TRIGGER calendar_days_set_updated_at BEFORE UPDATE ON calendar_days
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- The month view and the range queries all ask for a span of dates.
CREATE INDEX calendar_days_date_idx ON calendar_days (date);

-- The full day of this installation -----------------------------------------

-- Hours rather than a weekly total: the calendar answers which days are
-- worked, so a week's norm is a sum over its days. A stored weekly figure
-- would have to be reconciled with the calendar on every holiday, and the two
-- would disagree the first time a week had one.
--
-- Eight by default, which is what most installations mean by a full day. An
-- installation where it is not eight sets it once, in the same place the
-- privacy level lives, and the change is audited.
--
-- `numeric(4,2)` because 7.5 and 6.6 are real answers, and float hours in a
-- timesheet turn into a cent of rounding argument at the end of the month.
ALTER TABLE settings ADD COLUMN standard_hours numeric(4,2) NOT NULL DEFAULT 8
    CHECK (standard_hours > 0 AND standard_hours <= 24);

-- What one person's full day is, as a share ---------------------------------

-- A share of the installation's day, not their own number of hours. Half time
-- is `0.5` and stays half time when the installation moves to a seven-hour
-- day or when a short day takes an hour off - which a stored personal figure
-- would not, and nobody would notice until the timesheet was already signed.
--
-- One is the default and the overwhelming case; zero is allowed for someone on
-- the books who is not expected to work any hours at all, so the dashboard
-- does not owe a norm it was never meant to have.
ALTER TABLE users ADD COLUMN work_rate numeric(4,3) NOT NULL DEFAULT 1
    CHECK (work_rate >= 0 AND work_rate <= 2);

-- What kind of day this was --------------------------------------------------

-- Sent by the agent with the day, from `kasl day off`. The server holds it
-- because the norm has to know: a day of leave owes nothing, and a dashboard
-- that counted it as a missed eight hours would report every holiday as a
-- team-wide collapse.
--
-- `work` is the default, which is what every agent shipped before the field
-- says by saying nothing (ADR 0004). The values are the ones an employee
-- would choose between, not a taxonomy: leave, illness, and a day simply not
-- worked.
CREATE TYPE workday_kind AS ENUM ('work', 'vacation', 'sick', 'day_off');

ALTER TABLE workdays ADD COLUMN kind workday_kind NOT NULL DEFAULT 'work';
