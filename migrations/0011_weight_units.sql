-- How a household reads weights on its shopping list, and foods it reads in other units.

ALTER TABLE households ADD COLUMN weight_unit text NOT NULL DEFAULT 'lb_oz' CHECK (weight_unit IN ('lb_oz', 'oz', 'lb', 'metric'));

CREATE TABLE food_weight_units (
    household_id uuid NOT NULL REFERENCES households(id) ON DELETE CASCADE,
    food_id      uuid NOT NULL REFERENCES foods(id) ON DELETE CASCADE,
    weight_unit  text NOT NULL CHECK (weight_unit IN ('lb_oz', 'oz', 'lb', 'metric')),
    PRIMARY KEY (household_id, food_id)
);
