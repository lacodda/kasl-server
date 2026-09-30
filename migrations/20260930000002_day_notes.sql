-- Notes on a day: a manager's word on one of an employee's days.
--
-- "Your day off on Friday is approved", "thanks for staying for the release" -
-- the few sentences that otherwise travel through a chat nobody can find
-- again, and that belong next to the day they are about (ADR 0021).
--
-- **On a date, not on a workday.** The note most worth writing is about a day
-- that has not happened: leave approved ahead of time has no workday row to
-- hang from, and a day the agent never reported has none either. A note is
-- keyed the way the employee's own days are - the person and their local
-- calendar date - so it lands on the same line of the week whether or not
-- kasl ever sends that day.
--
-- **Written once.** There is no edit: the person may already have read it,
-- on their machine or in the inbox, and a note that changes under them is a
-- note they cannot rely on. A correction is a second note. A note written
-- about the wrong person can be withdrawn, and withdrawing takes the words
-- out - the row stays, so the notice that announced it can say it was
-- withdrawn, but the text is gone from the only place it was kept.
CREATE TABLE day_notes (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Whose day. Cascades like their days do: an account removed from the
    -- installation takes what was written about it along.
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- The employee's local calendar date, the same one `workdays.date` holds.
    date         date        NOT NULL,
    -- Who wrote it. Kept when the author's account goes: the note is still
    -- the employee's to read, signed by nobody the installation still knows.
    author_id    uuid        REFERENCES users (id) ON DELETE SET NULL,
    -- The words, and the only copy of them. The notice that announced the
    -- note reads them from here rather than carrying its own, so withdrawing
    -- the note cannot leave them behind anywhere (ADR 0021).
    text         text,
    created_at   timestamptz NOT NULL DEFAULT now(),
    -- When it was withdrawn. The text goes at the same moment.
    withdrawn_at timestamptz,

    -- Words exactly while the note stands: a standing note with nothing in it
    -- would be a notice with no sentence, and a withdrawn one still holding
    -- its text would be a withdrawal in name only.
    CONSTRAINT day_notes_text_until_withdrawn CHECK ((withdrawn_at IS NULL) = (text IS NOT NULL)),
    -- A note, not a letter. The handler says so first, with the limit in the
    -- message; this is the floor under it.
    CONSTRAINT day_notes_text_is_a_note CHECK (text IS NULL OR (btrim(text) <> '' AND char_length(text) <= 1000)),
    -- A note is from somebody to somebody. On your own day it would tell
    -- nobody anything, and the notice it raises would go to its own author.
    CONSTRAINT day_notes_not_on_own_day CHECK (author_id <> user_id)
);

-- The week of one person, which is how every screen asks for them.
CREATE INDEX day_notes_user_id_date_idx ON day_notes (user_id, date);

-- The notice that tells the person. One note is told once, like one alert.
ALTER TABLE notifications ADD COLUMN note_id uuid UNIQUE REFERENCES day_notes (id) ON DELETE CASCADE;

-- A reference exists exactly when the kind has one - the rule the alert and
-- the agent columns already follow. A `note.added` pointing nowhere could
-- never be withdrawn.
ALTER TABLE notifications ADD CONSTRAINT notifications_note_matches_kind CHECK ((kind = 'note.added') = (note_id IS NOT NULL));
