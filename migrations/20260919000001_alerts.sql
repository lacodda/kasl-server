-- Alerts: what the server noticed *now*, and what a person did about it.
--
-- Signals are not stored, deliberately (ADR 0016): they are a function of the
-- days already in the database, and a table of them would be a second copy of
-- a derived fact. An alert looks like a signal and is a different kind of
-- object, because two of the things it carries are not derivable from any day:
--
--   * **when it started.** "Lukas has been quiet since 09:15" is not in the
--     workdays - the absence of rows has no timestamp. Recomputing on every
--     read can say a thing is true; only a row can say since when.
--   * **what was done about it.** A manager who has seen an alert and decided
--     it is a holiday needs it to stop shouting. Nothing derived from the
--     days can remember that, and an alert that cannot be dismissed teaches
--     people to ignore the column - the lesson `unknown` taught in ADR 0014.
--
-- So what is stored here is not a conclusion but an **observation event**: at
-- this moment this rule was true about this person, and here is what has
-- happened to it since. The rule itself is still computed from the days on
-- every sweep; the sweep then reconciles what *should* be open against what
-- *is*, which is why one quiet week is one row rather than seven.

-- Which rule fired --------------------------------------------------------
--
-- A closed set, like every other enum in this schema: the screen branches on
-- it and the webhooks of v0.23 will route on it, and free text would make a
-- typo a fourth rule that nothing renders and nothing delivers.
--
-- Three, and each one names a fact the server actually holds:
--
--   * `no_agent_data` - nothing has arrived from this person's machines for
--     longer than the threshold. About *now*, in hours - which is what makes
--     it a different thing from the `no_data` signal, that measures a person's
--     own rhythm in whole weeks and cannot speak before Monday.
--   * `overwork` - a finished day ran well past what that person owed it.
--     Only sayable since the production calendar (ADR 0017): before the norm,
--     a threshold in hours would have been this product asserting what a
--     working day is on somebody else's team, which ADR 0016 refused.
--   * `day_not_closed` - a day is still open long after the hours in it
--     stopped being plausible. Usually kasl left running overnight, and the
--     day it produces is wrong in a way that quietly poisons a week's total.
CREATE TYPE alert_rule AS ENUM ('no_agent_data', 'overwork', 'day_not_closed');

-- What a person did with it ------------------------------------------------
--
-- `open` is every alert that is still true and still unattended. The other two
-- are the two different ways one stops needing attention, and they are *not*
-- the same fact:
--
--   * `acknowledged` - a person looked and decided it needs no action. The
--     condition may well still be true; the alert is answered, not gone.
--   * `resolved` - the condition itself stopped being true, and nobody had to
--     do anything. The agent came back, the day got closed.
--
-- Collapsing them into one flag would lose the only question worth asking of
-- this table later: how much of what the server shouted about was real.
CREATE TYPE alert_state AS ENUM ('open', 'acknowledged', 'resolved');

CREATE TABLE alerts (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Who it is about. Cascades: an account removed from the installation
    -- takes its alerts with it, exactly as it takes its days.
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    rule         alert_rule  NOT NULL,
    state        alert_state NOT NULL DEFAULT 'open',
    -- When the condition was first seen to be true. Not `created_at`: the two
    -- are the same on a live server and differ by a restart's worth of catch-up
    -- on a server that was down, and the honest one is when it became true.
    fired_at     timestamptz NOT NULL DEFAULT now(),
    -- When the condition stopped being true, and when somebody answered it.
    -- Both null while it is open; either one moves `state` off `open`.
    resolved_at  timestamptz,
    acknowledged_at timestamptz,
    -- Who answered it. Null on one the server resolved by itself, which is
    -- how "it went away" is told from "somebody decided it was fine".
    acknowledged_by uuid REFERENCES users (id) ON DELETE SET NULL,

    -- The figures the rule fired on -----------------------------------------
    --
    -- Carried on the row rather than recomputed for display, because by the
    -- time anybody reads it the numbers have moved on: an alert raised at nine
    -- hours of silence reads "nine hours" forever, and re-deriving it would
    -- make yesterday's alert quote today's arithmetic. The same reason the
    -- signals carry theirs (ADR 0016) - a figure travels with the statement it
    -- justifies, so the screen can say what happened rather than what it means.

    -- What was measured, in seconds: hours of silence, seconds worked, how
    -- long the day has been open. One column and not three, because every rule
    -- here measures a duration and a rule that did not would need its own
    -- column anyway.
    observed_seconds bigint NOT NULL,
    -- What it was measured against, in seconds: the threshold, or the norm the
    -- threshold came from. Null where a rule has no second number to show.
    --
    -- Both are stored because a percentage cannot be un-divided: "11 h against
    -- a norm of 8" and "138%" are not the same sentence, and only one of them
    -- lets a reader disagree with the arithmetic (ADR 0017).
    against_seconds  bigint,
    -- The date the alert is about, when it is about one. Null for
    -- `no_agent_data`, which is about a silence rather than about a day.
    subject_date     date,

    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now(),

    -- The two stamps and the state cannot disagree. A row that says `resolved`
    -- with no `resolved_at` would make "since when" unanswerable, and a row
    -- that says `open` with one would be a resolution nobody applied.
    CONSTRAINT alerts_state_matches_stamps CHECK (
        (state = 'open'         AND resolved_at IS NULL AND acknowledged_at IS NULL)
     OR (state = 'acknowledged' AND acknowledged_at IS NOT NULL)
     OR (state = 'resolved'     AND resolved_at IS NOT NULL)
    )
);

CREATE TRIGGER alerts_set_updated_at BEFORE UPDATE ON alerts
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- One open alert per person per rule, and the database is what says so.
--
-- The sweep reconciles rather than inserts, so in principle it never writes a
-- second one - but "in principle" is what a partial unique index is for. Two
-- sweeps overlapping (a restart mid-run, a second instance behind a load
-- balancer) would otherwise give a manager the same quiet agent twice, and
-- from then on every sweep would keep both alive.
--
-- Partial, on `open` alone: a person may have been quiet in March and quiet
-- again in July, and the resolved March row has to stay.
CREATE UNIQUE INDEX alerts_one_open_per_rule ON alerts (user_id, rule) WHERE state = 'open';

-- The manager's feed: the open ones, newest first, across everyone they see.
CREATE INDEX alerts_open_fired_at_idx ON alerts (fired_at DESC) WHERE state = 'open';

-- The drill-down, and the sweep's own lookup of what is already open.
CREATE INDEX alerts_user_id_fired_at_idx ON alerts (user_id, fired_at DESC);

-- The thresholds ------------------------------------------------------------
--
-- On `settings`, next to `standard_hours` and the privacy level, because they
-- are the same kind of fact: one decision, made once by whoever runs this
-- installation, audited when it changes (ADR 0011, ADR 0017).
--
-- These three *are* settings and not hard-coded choices, which is the opposite
-- of what ADR 0016 decided for the signal thresholds - and for a reason that
-- distinguishes them. A signal is read by the person who opened the page, and
-- a sensitivity slider there is a knob on an opinion. An alert interrupts
-- somebody, and how much silence is worth interrupting over genuinely differs
-- between a team on one timezone and a team on four. What must not be a
-- setting is the *rule*; how far it is willing to go before speaking is the
-- operator's to say.

-- Hours of silence from every one of a person's agents before it is an alert.
--
-- Twelve by default: long enough to sleep through without a dashboard filling
-- up overnight, short enough that a machine that died on Monday morning is
-- noticed on Monday rather than on Tuesday. The `no_data` signal, which works
-- in whole weeks, cannot say anything at all this side of the weekend.
ALTER TABLE settings ADD COLUMN alert_silence_hours int NOT NULL DEFAULT 12
    CHECK (alert_silence_hours > 0 AND alert_silence_hours <= 720);

-- How far past their own norm a finished day has to run to be overwork, as a
-- share of that norm.
--
-- A share and not a number of hours, so it means the same thing for somebody
-- on half time: 1.5 is "half again as long as that person owed", which is
-- eleven hours against an eight-hour norm and six against a four-hour one. An
-- installation-wide "over ten hours" would call a part-timer's ordinary week
-- fine and never notice it doubling.
ALTER TABLE settings ADD COLUMN alert_overwork_factor numeric(4,2) NOT NULL DEFAULT 1.5
    CHECK (alert_overwork_factor > 1 AND alert_overwork_factor <= 5);

-- Hours a day may stay open before the server says so.
--
-- Sixteen: past any plausible day including the long ones, and short enough
-- that a laptop left running on Friday is an alert on Saturday morning rather
-- than a 60-hour Monday nobody can now reconstruct. Not derived from the norm
-- like overwork is - an open day has no total yet, so there is nothing to
-- compare with a norm; what is wrong with it is the wall clock.
ALTER TABLE settings ADD COLUMN alert_open_day_hours int NOT NULL DEFAULT 16
    CHECK (alert_open_day_hours > 0 AND alert_open_day_hours <= 168);
