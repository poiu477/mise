-- Who made each dish: the grouping job, or a person. Re-grouping only replaces grouped dishes.
ALTER TABLE dishes ADD COLUMN origin text NOT NULL DEFAULT 'grouped' CHECK (origin IN ('grouped', 'user'));
-- A person pinned this variant to its dish; grouping leaves it where it is.
ALTER TABLE variants ADD COLUMN dish_pinned boolean NOT NULL DEFAULT false;
CREATE INDEX dishes_slug ON dishes (slug);
