-- Addresses a dish used to have. Grouping renames a dish when its name changes, and a dish whose versions all
-- joined another one hands its address on; the old slug stays here so links and bookmarks still find the dish.
-- A slug a live dish takes is removed from this list, so a live dish always wins.
CREATE TABLE retired_slugs (
    slug        text PRIMARY KEY,
    dish_id     uuid NOT NULL REFERENCES dishes(id) ON DELETE CASCADE,
    retired_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX retired_slugs_dish ON retired_slugs (dish_id);
