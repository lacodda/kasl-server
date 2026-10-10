-- What a day comes to, with what was paused in it (ADR 0023).
--
-- The view arrived in 0.26 with the worked figure only, because the reports
-- compared nothing else. The team table and the exports also say what was
-- paused, and that figure was still derived a second time in `team.rs`, with
-- the same CASE copied beside the view's. The paused seconds the worked figure
-- is computed from are now a column of their own, so both read one rule.
--
-- `CREATE OR REPLACE VIEW` may only add columns at the end; the existing ones
-- keep their names, types and order.
CREATE OR REPLACE VIEW workday_figures AS
SELECT w.id,
       w.user_id,
       w.date,
       w.kind,
       w.started_at,
       w.ended_at,
       CASE WHEN w.ended_at IS NULL THEN NULL
            ELSE greatest(floor(extract(epoch FROM (w.ended_at - w.started_at)))::bigint - paused.seconds, 0)
       END AS worked_seconds,
       -- Stored pauses where they exist, the day's own totals where a narrower
       -- policy summarized them away (ADR 0011). Answered for an open day too:
       -- the breaks already taken are on the record, the total is not.
       paused.seconds AS paused_seconds
FROM workdays w
CROSS JOIN LATERAL (
    SELECT CASE
        WHEN EXISTS (SELECT 1 FROM pauses p WHERE p.workday_id = w.id)
        THEN (SELECT coalesce(sum(p.duration_seconds), 0)::bigint FROM pauses p WHERE p.workday_id = w.id)
        ELSE coalesce(w.paused_seconds, 0)::bigint
    END AS seconds
) AS paused;
