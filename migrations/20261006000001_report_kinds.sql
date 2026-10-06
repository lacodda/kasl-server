-- A day approved and a day returned are the sixth and seventh things the
-- server tells a person (ADR 0022): new values of the kind, not a channel of
-- their own (ADR 0020).
--
-- Their own migration because PostgreSQL will not let a value added to an enum
-- be used in the transaction that added it, and the next migration's
-- constraint names one of them. sqlx runs each file in a transaction of its
-- own, so by the time the next one starts this one is committed.
ALTER TYPE notification_kind ADD VALUE 'report.approved';
ALTER TYPE notification_kind ADD VALUE 'report.returned';
