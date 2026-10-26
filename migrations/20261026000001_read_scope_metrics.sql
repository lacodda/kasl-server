-- A read token for the server's own health: `/metrics` (ADR 0026).
--
-- A scope beside `hours` rather than a credential of its own: a metrics token
-- is issued, listed, revoked and audited exactly as a dashboard's is, and only
-- what it reads differs. Who may hold it - an administrator - is decided in
-- code, on issue and on every request, by the same rule (`ReadScope::held_by`).
ALTER TYPE read_scope ADD VALUE 'metrics';
