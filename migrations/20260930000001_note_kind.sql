-- A manager's note on a day is the fifth thing the server tells a person
-- (ADR 0021): a new value of the kind, not a channel of its own (ADR 0020).
--
-- Its own migration because PostgreSQL will not let a value added to an enum
-- be used in the transaction that added it, and the next migration's
-- constraint names it. sqlx runs each file in a transaction of its own, so by
-- the time the next one starts this one is committed.
ALTER TYPE notification_kind ADD VALUE 'note.added';
