-- Two rules several queries share, kept in the database beside `visible_to` so each query that uses one is a single
-- plain statement the compiler can check against the schema (T35). Both are one SQL expression, so the planner
-- inlines them as it did the text they replace.

-- Whether `household` may know of food `f`: every food mise was given or a person edited, and of the foods matching
-- made, those a version the household can see uses. Categories are shelves, not foods to pick. Food search, reading
-- lines in the editor and checking a saved recipe's foods all use it, so a food named only in another household's
-- private recipe is never shown or accepted.
CREATE FUNCTION known_food(f foods, household uuid) RETURNS boolean
    LANGUAGE sql STABLE PARALLEL SAFE
    AS $$
    SELECT f.kind <> 'category'
        AND (f.origin <> 'matched' OR EXISTS (
            SELECT 1 FROM ingredient_lines il JOIN variants v ON v.id = il.variant_id
            WHERE il.food_id = f.id AND visible_to(v, household)))
    $$;

-- The version dish `d` opens on for `household`: its default when the household can see it, else the one with steps
-- published last; null when the household can see none of its versions.
CREATE FUNCTION default_version(d dishes, household uuid) RETURNS uuid
    LANGUAGE sql STABLE PARALLEL SAFE
    AS $$
    SELECT v.id FROM variants v
    WHERE v.dish_id = d.id AND visible_to(v, household)
    ORDER BY v.id IS NOT DISTINCT FROM d.default_variant_id DESC, v.has_steps DESC, v.published_at DESC NULLS LAST, v.id
    LIMIT 1
    $$;
