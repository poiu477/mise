-- What distinguishes each variant within its dish (protein, portion, version). A dish's options are the
-- attributes that vary across its variants; a combination is offered only if some variant has it.
CREATE TABLE variant_attributes (
    variant_id  uuid NOT NULL REFERENCES variants(id) ON DELETE CASCADE,
    key         text NOT NULL,              -- 'protein', 'portion', 'version'
    value       text NOT NULL,
    PRIMARY KEY (variant_id, key)
);
CREATE INDEX variant_attributes_key ON variant_attributes (key, value);

CREATE TABLE dish_options (
    dish_id       uuid NOT NULL REFERENCES dishes(id) ON DELETE CASCADE,
    key           text NOT NULL,
    label         text NOT NULL,            -- "Protein", "Portion", "Version"
    position      int  NOT NULL,
    choices       jsonb NOT NULL,           -- [{value, label, variants, years}]
    default_value text,                     -- the default variant's value
    PRIMARY KEY (dish_id, key)
);
