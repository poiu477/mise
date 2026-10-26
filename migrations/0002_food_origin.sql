-- Who created each food and alias, so re-running matching never touches knowledge or a person's edits.
ALTER TABLE foods ADD COLUMN origin text NOT NULL DEFAULT 'matched' CHECK (origin IN ('knowledge', 'matched', 'user'));
ALTER TABLE food_aliases ADD COLUMN origin text NOT NULL DEFAULT 'matched' CHECK (origin IN ('knowledge', 'matched', 'user'));

-- Foods that carry facts came from imported knowledge, as did their category paths.
UPDATE foods SET origin = 'knowledge'
WHERE kind = 'category' OR EXISTS (SELECT 1 FROM food_facts ff WHERE ff.food_id = foods.id);
UPDATE food_aliases a SET origin = 'knowledge' FROM foods f WHERE f.id = a.food_id AND f.origin = 'knowledge';
