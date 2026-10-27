-- Step text parsed into parts (mise_core::step_amounts::Template): amounts with their per-yield values and the
-- ingredient line they measure, yield-dependent wording and notes. Filled by `mise parse-steps`; the published
-- text stays in `text`.
ALTER TABLE steps ALTER COLUMN template TYPE jsonb USING template::jsonb;
