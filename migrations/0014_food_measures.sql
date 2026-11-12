-- What a food's amounts convert by on the shopping list, so a week's needs add up across units: its density (what a
-- volume of it weighs) and the size of one item of it (one clove, one onion). A food without them keeps amounts in
-- units that don't convert apart, joined with "+". Kinds of a food use the food's own when they have none.
--
-- Shopping marks are untouched: they were already kept per food (`item_key` is the food's id), which is what lets a
-- line that merges kinds of a food, or splits again, keep its ticks.

-- One measure of a food. A density gives both a volume and its weight; an item size gives one or both for the item
-- word recipes count the food in ("clove"), singular.
CREATE TABLE food_measures (
    id         uuid PRIMARY KEY,
    food_id    uuid NOT NULL REFERENCES foods(id) ON DELETE CASCADE,
    kind       text NOT NULL CHECK (kind IN ('density', 'item')),
    item       text NOT NULL DEFAULT '',
    ml         numeric CHECK (ml > 0),
    grams      numeric CHECK (grams > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (food_id, kind, item),
    CHECK (
        (kind = 'density' AND item = '' AND ml IS NOT NULL AND grams IS NOT NULL)
        OR (kind = 'item' AND length(item) BETWEEN 1 AND 50 AND (ml IS NOT NULL OR grams IS NOT NULL))
    )
);
