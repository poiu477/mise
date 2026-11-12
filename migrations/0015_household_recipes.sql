-- Recipes that belong to a household. Imports, the editor and forks make versions one household owns; seed and bundle
-- versions belong to no household and every account sees them, as before.
--
-- `visibility` says how far an owned version is shared. Only 'private' (its own household) exists so far; T31 adds
-- 'partners' and 'install' by widening the check and `visible_to` below. A version with no household is everyone's,
-- whatever its visibility says.
ALTER TABLE variants
    ADD COLUMN household_id uuid REFERENCES households(id),
    ADD COLUMN visibility   text NOT NULL DEFAULT 'private' CONSTRAINT variants_visibility_check CHECK (visibility IN ('private'));
CREATE INDEX variants_household ON variants (household_id) WHERE household_id IS NOT NULL;

-- Whether `household` can see version `v`: the one rule every query that reads recipes for a household goes through,
-- so widening sharing later changes it here and nowhere else. A plain SQL function, so the planner inlines it.
CREATE FUNCTION visible_to(v variants, household uuid) RETURNS boolean
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    AS $$ SELECT v.household_id IS NULL OR v.household_id = household $$;
