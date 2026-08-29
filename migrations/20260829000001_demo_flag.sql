-- Whether this installation holds the demo's fictional team (ADR 0013).
--
-- Stored in the database rather than read from the environment each time:
-- the flag has to survive the operator dropping `KASL_DEMO` from their file,
-- or the web UI would stop saying "nothing here is real" over data that is
-- still invented. It also lets a start with `KASL_DEMO` set tell a database
-- it seeded itself from one that holds somebody's actual team.
ALTER TABLE settings ADD COLUMN demo bool NOT NULL DEFAULT false;
