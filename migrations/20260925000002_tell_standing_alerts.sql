-- Tell people about the alerts that were already standing when notifications
-- arrived.
--
-- A notice is written in the transaction that raises an alert (ADR 0020), so
-- an alert raised before the table existed was never told to the person it is
-- about. On the first installation upgraded to 0.24.0 that was an open day and
-- a silence - the case the whole milestone exists for, and the person still
-- not told. The sweep never raises them again: they are open, and it
-- reconciles rather than inserts (ADR 0018).
--
-- Standing is `resolved_at IS NULL`: open, or answered by a manager while the
-- condition still holds. Both were raised, and a notice says "your manager was
-- told", which is true of either. What has resolved is over and is not told.
--
-- The payload is the shape `Facts` reads back - `{"alert": AlertPayload}` -
-- with the figures the alert fired on, never recomputed (ADR 0018). Written
-- now, so the machines of these people toast them once, and in the order the
-- alerts fired.
INSERT INTO notifications (user_id, kind, alert_id, payload)
SELECT al.user_id,
       'alert.raised',
       al.id,
       jsonb_build_object('alert', jsonb_build_object(
           'id',               al.id,
           'rule',             al.rule,
           'observed_seconds', al.observed_seconds,
           'against_seconds',  al.against_seconds,
           'subject_date',     al.subject_date,
           'fired_at',         al.fired_at))
FROM alerts al
WHERE al.resolved_at IS NULL
ORDER BY al.fired_at, al.id
-- One alert, one notice: a notice the new code already wrote for it wins.
ON CONFLICT (alert_id) DO NOTHING;
