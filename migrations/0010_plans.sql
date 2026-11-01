-- The household's weekly plan and what it does to the shopping list. The list itself is worked out from the plan each
-- time, so a change to the plan shows at once; only what people do to it is stored here.
CREATE TABLE plan_entries (
    id           uuid PRIMARY KEY,
    household_id uuid NOT NULL REFERENCES households(id) ON DELETE CASCADE,
    week         date NOT NULL,                  -- the Monday
    variant_id   uuid NOT NULL REFERENCES variants(id) ON DELETE CASCADE,
    servings     numeric NOT NULL CHECK (servings > 0),
    added_by     uuid REFERENCES users(id) ON DELETE SET NULL,
    created_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX plan_entries_week ON plan_entries (household_id, week);

-- An item on a week's list ticked off or taken off it. The key is the food's id, or the name for an unmatched line.
CREATE TABLE shopping_marks (
    household_id uuid NOT NULL REFERENCES households(id) ON DELETE CASCADE,
    week         date NOT NULL,
    item_key     text NOT NULL,
    checked      boolean NOT NULL DEFAULT false,
    hidden       boolean NOT NULL DEFAULT false,
    PRIMARY KEY (household_id, week, item_key)
);

-- Things added to a week's list by hand.
CREATE TABLE shopping_extras (
    id           uuid PRIMARY KEY,
    household_id uuid NOT NULL REFERENCES households(id) ON DELETE CASCADE,
    week         date NOT NULL,
    name         text NOT NULL,
    checked      boolean NOT NULL DEFAULT false,
    created_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX shopping_extras_week ON shopping_extras (household_id, week);

-- Foods this household always has, left off every list.
CREATE TABLE pantry_staples (
    household_id uuid NOT NULL REFERENCES households(id) ON DELETE CASCADE,
    food_id      uuid NOT NULL REFERENCES foods(id) ON DELETE CASCADE,
    PRIMARY KEY (household_id, food_id)
);
