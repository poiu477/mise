-- Sharing between households (T31). A version a household owns can be shared with the households it has added as
-- partners, or with the whole install; each household has a default for the recipes and forks it makes, and any one
-- can say otherwise. An admin caps the most open level on the install (SHARING_MAX_LEVEL, which the server writes
-- to `sharing_cap` when it starts); a cap below what an item asks for hides it from those the cap leaves out, as if
-- it asked for no more than the cap.

ALTER TABLE variants DROP CONSTRAINT variants_visibility_check;
ALTER TABLE variants ADD CONSTRAINT variants_visibility_check CHECK (visibility IN ('private', 'partners', 'install'));

ALTER TABLE households
    ADD COLUMN default_visibility text NOT NULL DEFAULT 'private'
        CHECK (default_visibility IN ('private', 'partners', 'install'));

-- `partner_id` sees what `household_id` shares with its partners. Each household chooses whom it shares with, so a
-- link runs one way: the other household shares back only once it adds this one too.
CREATE TABLE household_partners (
    household_id uuid NOT NULL REFERENCES households(id) ON DELETE CASCADE,
    partner_id   uuid NOT NULL REFERENCES households(id) ON DELETE CASCADE,
    created_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (household_id, partner_id),
    CHECK (household_id <> partner_id)
);
CREATE INDEX household_partners_partner ON household_partners (partner_id);

-- The install's cap: one row, or none, which means no cap below `install`.
CREATE TABLE sharing_cap (
    only_row  boolean PRIMARY KEY DEFAULT true CHECK (only_row),
    max_level text NOT NULL CHECK (max_level IN ('private', 'partners', 'install'))
);

-- Whether `household` can see version `v`: still the one rule every query that reads recipes for a household goes
-- through. A version with no household is everyone's. An owned one is its household's, its partners' when it is
-- shared with partners or the install and the cap allows partners, and everyone's when it is shared with the
-- install and the cap allows that. STABLE now, since it reads the partner links and the cap; still one plain SQL
-- expression, so the planner inlines it.
CREATE OR REPLACE FUNCTION visible_to(v variants, household uuid) RETURNS boolean
    LANGUAGE sql STABLE PARALLEL SAFE
    AS $$
    SELECT v.household_id IS NULL
        OR v.household_id = household
        OR (v.visibility = 'install'
            AND coalesce((SELECT c.max_level FROM sharing_cap c), 'install') = 'install')
        OR (v.visibility IN ('partners', 'install')
            AND coalesce((SELECT c.max_level FROM sharing_cap c), 'install') IN ('partners', 'install')
            AND EXISTS (SELECT 1 FROM household_partners p
                        WHERE p.household_id = v.household_id AND p.partner_id = household))
    $$;
