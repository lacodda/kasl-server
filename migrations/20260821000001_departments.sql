-- Departments: the scope a manager's authority is measured in.
--
-- Until now `role = 'manager'` meant "may read the whole company", which is
-- only tolerable while a company is small enough to fit on one screen. A
-- department gives that role a boundary (ADR 0009).
CREATE TABLE departments (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name       text        NOT NULL,
    -- Who runs it. Null for a department between managers - a real state, and
    -- one an admin should be able to see rather than being forced to invent a
    -- placeholder head for.
    --
    -- ON DELETE SET NULL rather than CASCADE: users are deactivated, never
    -- deleted, but if a row ever does go the department must survive its
    -- manager. Losing a department would orphan everyone in it.
    manager_id uuid        REFERENCES users (id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

-- Names are how people refer to departments out loud, so two called "Sales"
-- would be a support call. Case-insensitive, like emails.
CREATE UNIQUE INDEX departments_name_key ON departments (lower(name));
CREATE INDEX departments_manager_id_idx ON departments (manager_id);

CREATE TRIGGER departments_set_updated_at BEFORE UPDATE ON departments
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Nullable on purpose. Someone just added, the administrator themselves, and
-- every account that exists today have no department, and a person nobody has
-- filed yet is visible to the admin alone - which is noticed immediately,
-- unlike a leak to every manager in the company.
--
-- ON DELETE SET NULL so removing a department leaves its people unfiled rather
-- than deleting them along with it.
ALTER TABLE users ADD COLUMN department_id uuid REFERENCES departments (id) ON DELETE SET NULL;

CREATE INDEX users_department_id_idx ON users (department_id);
