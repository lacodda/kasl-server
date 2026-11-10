-- Overtime: what an hour beyond the ordinary pays (ADR 0028).
--
-- The rules are part of a version of the terms, not a table beside it. A
-- version is history that is never rewritten (ADR 0027), and rules held in
-- rows of their own could be added to a version after money had been computed
-- from it; as columns of the row they are as fixed as its rate. Each rule is a
-- ladder of at most two steps, which is every law the finance module was
-- planned against: the first multiplier from a threshold, and an optional
-- second, higher one some hours past it (California: 1.5 after 8 hours, 2
-- after 12; Russia: 1.5 for the first two hours past the norm, 2 after).
--
-- A threshold is either fixed hours worked in the span or, when left null
-- under a multiplier, the span's own norm - the person's day as the calendar
-- and their share of a full day ask for it, or their week less any leave.
--
-- Adding nullable columns rewrites no row and fires no trigger, so the
-- versions recorded before this keep exactly what they said: no overtime.

ALTER TABLE pay_terms
    -- Per day worked. `daily_after_hours` null with a multiplier set means
    -- past the date's norm.
    ADD COLUMN daily_after_hours numeric(4, 2) CHECK (daily_after_hours > 0 AND daily_after_hours <= 24),
    ADD COLUMN daily_multiplier numeric(6, 4) CHECK (daily_multiplier > 1 AND daily_multiplier <= 10),
    -- The second step: hours past the first step's start, and what they pay.
    ADD COLUMN daily_then_after_hours numeric(4, 2) CHECK (daily_then_after_hours > 0 AND daily_then_after_hours <= 24),
    ADD COLUMN daily_then_multiplier numeric(6, 4) CHECK (daily_then_multiplier > 1 AND daily_then_multiplier <= 10),
    -- Per Monday-to-Sunday week, counting only the hours no daily step or rest
    -- day already paid more for. `weekly_after_hours` null with a multiplier
    -- set means past the week's norm.
    ADD COLUMN weekly_after_hours numeric(5, 2) CHECK (weekly_after_hours > 0 AND weekly_after_hours <= 168),
    ADD COLUMN weekly_multiplier numeric(6, 4) CHECK (weekly_multiplier > 1 AND weekly_multiplier <= 10),
    ADD COLUMN weekly_then_after_hours numeric(5, 2) CHECK (weekly_then_after_hours > 0 AND weekly_then_after_hours <= 168),
    ADD COLUMN weekly_then_multiplier numeric(6, 4) CHECK (weekly_then_multiplier > 1 AND weekly_then_multiplier <= 10),
    -- Every hour of a date the calendar asks nothing of - a weekend not moved
    -- into the week, a holiday.
    ADD COLUMN rest_day_multiplier numeric(6, 4) CHECK (rest_day_multiplier > 1 AND rest_day_multiplier <= 10),
    -- A threshold or a second step without the multiplier they belong to is a
    -- rule that says nothing, and would read as one that does.
    ADD CONSTRAINT pay_terms_daily_rule_whole CHECK (
        daily_multiplier IS NOT NULL
        OR (daily_after_hours IS NULL AND daily_then_after_hours IS NULL AND daily_then_multiplier IS NULL)
    ),
    ADD CONSTRAINT pay_terms_weekly_rule_whole CHECK (
        weekly_multiplier IS NOT NULL
        OR (weekly_after_hours IS NULL AND weekly_then_after_hours IS NULL AND weekly_then_multiplier IS NULL)
    ),
    -- A second step is a place and a multiplier, together, and pays more than
    -- the first: a ladder that went down would pay the eleventh hour less than
    -- the ninth.
    ADD CONSTRAINT pay_terms_daily_second_step CHECK (
        (daily_then_after_hours IS NULL) = (daily_then_multiplier IS NULL)
        AND (daily_then_multiplier IS NULL OR daily_then_multiplier > daily_multiplier)
    ),
    ADD CONSTRAINT pay_terms_weekly_second_step CHECK (
        (weekly_then_after_hours IS NULL) = (weekly_then_multiplier IS NULL)
        AND (weekly_then_multiplier IS NULL OR weekly_then_multiplier > weekly_multiplier)
    ),
    -- A fixed ladder stays inside its span: a second step at the 26th hour of
    -- a day is a step nobody can reach.
    ADD CONSTRAINT pay_terms_daily_ladder_fits CHECK (
        daily_after_hours IS NULL OR daily_after_hours + coalesce(daily_then_after_hours, 0) <= 24
    ),
    ADD CONSTRAINT pay_terms_weekly_ladder_fits CHECK (
        weekly_after_hours IS NULL OR weekly_after_hours + coalesce(weekly_then_after_hours, 0) <= 168
    );
