-- Reports, and their approval: the person says a day is finished, and - where
-- the installation asks for it - a manager approves it (ADR 0022).

-- The placeholder goes ------------------------------------------------------------
--
-- The first schema carried a `reports` table for the day this arrived: "a
-- report is an event, not a second copy of the day ... what approval and the
-- late / missing report signals are built on". Nothing ever wrote to it, and
-- the shape it guessed does not hold: one row per day (a report sent again
-- after a correction is a second event, not an edit of the first), a monthly
-- kind (closing a pay period is its own milestone), an agent-computed
-- productivity figure (the server computes the day's figures itself).
--
-- Refused rather than dropped if somebody did write to it by hand: losing rows
-- nobody can name is the one outcome a migration must not choose silently.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM reports) THEN
        RAISE EXCEPTION 'the reports table holds rows, but no version of kasl-server ever wrote to it; move them aside and start again';
    END IF;
END
$$;

DROP TABLE reports;
DROP TYPE report_kind;

-- What a day comes to ---------------------------------------------------------
--
-- A report is the person putting their name to a day's figures, and an
-- approval is a manager putting theirs to the same figures. Whether a report
-- still describes the day is a comparison between what was reported and what
-- the day comes to now - so "what the day comes to" needs one definition, in
-- the one place both sides of the comparison read it from.
--
-- Worked is the span minus what was paused; paused is the stored pauses where
-- they exist and the day's own totals where a narrower policy summarized them
-- away (ADR 0011) - one or the other, never both. An open day has no total.
-- Whole seconds, rounded down, which is what the days answer says
-- (`me::Day::worked_seconds`); the test suite holds the two to each other.
CREATE VIEW workday_figures AS
SELECT w.id,
       w.user_id,
       w.date,
       w.kind,
       w.started_at,
       w.ended_at,
       CASE WHEN w.ended_at IS NULL THEN NULL
            ELSE greatest(floor(extract(epoch FROM (w.ended_at - w.started_at)))::bigint - paused.seconds, 0)
       END AS worked_seconds
FROM workdays w
CROSS JOIN LATERAL (
    SELECT CASE
        WHEN EXISTS (SELECT 1 FROM pauses p WHERE p.workday_id = w.id)
        THEN (SELECT coalesce(sum(p.duration_seconds), 0)::bigint FROM pauses p WHERE p.workday_id = w.id)
        ELSE coalesce(w.paused_seconds, 0)::bigint
    END AS seconds
) AS paused;

-- Reports ---------------------------------------------------------------------
--
-- **A row is one report: an event, never edited.** The person said, at this
-- moment, that this day came to these figures. Corrected in kasl and reported
-- again, it is a second row - the first stays, with whatever a manager said
-- about it. What a day's report says now is its newest row.
--
-- **The figures are stored, the status is not.** Whether a report still
-- describes its day is read by comparing these figures with the day's, every
-- time: kasl remains the source of truth for the day (ADR 0004), so a day
-- re-sent after it was approved is stored as sent, and the approval simply
-- stops covering it. A status column would be a second copy of that
-- comparison, and every path that writes a day would have to remember to
-- update it.

-- What a manager decided. Two answers, both about the figures in the row.
CREATE TYPE report_review AS ENUM ('approved', 'returned');

CREATE TABLE reports (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Whose day. Cascades like their days do.
    user_id      uuid          NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- The employee's local calendar date, the one `workdays.date` holds.
    date         date          NOT NULL,

    -- The figures the person put their name to, as `workday_figures` gave
    -- them at that moment. Only a finished day is reported, so the end is
    -- never missing.
    kind         workday_kind  NOT NULL,
    started_at   timestamptz   NOT NULL,
    ended_at     timestamptz   NOT NULL,
    worked_seconds bigint      NOT NULL,

    -- The order of a day's reports. `clock_timestamp()` rather than `now()`:
    -- the report is written after the day is locked, and the time it is
    -- written is the time it took its place in the line.
    submitted_at timestamptz   NOT NULL DEFAULT clock_timestamp(),

    -- The answer, once there is one.
    review       report_review,
    -- Who gave it. Kept when that account goes: the answer stands, signed by
    -- nobody the installation still knows.
    reviewed_by  uuid          REFERENCES users (id) ON DELETE SET NULL,
    reviewed_at  timestamptz,
    -- Why it was returned: the manager's words, and the only copy of them -
    -- the notice that tells the person reads them from here (ADR 0022).
    reason       text,

    CONSTRAINT reports_end_after_start CHECK (ended_at >= started_at),
    CONSTRAINT reports_worked_not_negative CHECK (worked_seconds >= 0),
    -- An answer has a moment.
    CONSTRAINT reports_reviewed_when CHECK ((review IS NULL) = (reviewed_at IS NULL)),
    -- A day sent back says why, and only a day sent back says anything: an
    -- approval is of the figures, not of a sentence about them.
    CONSTRAINT reports_reason_when_returned CHECK ((review IS NOT DISTINCT FROM 'returned') = (reason IS NOT NULL)),
    -- A reason, not a letter. The handler says so first, with the limit in
    -- the message; this is the floor under it.
    CONSTRAINT reports_reason_is_a_reason CHECK (reason IS NULL OR (btrim(reason) <> '' AND char_length(reason) <= 1000)),
    -- Nobody approves their own day. A manager sees their own days, so the
    -- visibility rule alone would let them.
    CONSTRAINT reports_not_reviewed_by_self CHECK (reviewed_by <> user_id)
);

-- A day's reports, newest first: what every reader asks for.
CREATE INDEX reports_user_id_date_idx ON reports (user_id, date, submitted_at DESC);

-- Whether this installation asks for days to be approved ------------------------
--
-- Opt-in: an installation that never asked for approval must not grow a queue
-- of days waiting for somebody. Off, a report is a person saying a day is
-- finished; on, it is also a question to their manager.
ALTER TABLE settings ADD COLUMN day_approval boolean NOT NULL DEFAULT false;

-- The notice that a day came back ----------------------------------------------
--
-- Points at the report, which holds the reason. One report is returned once.
-- The approval notice points at nothing: one approval covers several days,
-- and what it says - who, which days, which figures - is all in its payload.
ALTER TABLE notifications ADD COLUMN report_id uuid UNIQUE REFERENCES reports (id) ON DELETE CASCADE;

-- A reference exists exactly when the kind has one - the rule the alert, the
-- agent and the note columns already follow.
ALTER TABLE notifications ADD CONSTRAINT notifications_report_matches_kind CHECK ((kind = 'report.returned') = (report_id IS NOT NULL));
