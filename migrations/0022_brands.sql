-- The brands food matching strips from ingredient names ("Old Bay Seasoning" is Old Bay's seasoning), kept as data
-- so curation can add or remove one without a release (T36). Seeded with the 42 mise-core listed until now.
CREATE TABLE brands (
    -- As `normalize_name` writes it: lower case, words single-spaced, apostrophes kept ("frank's redhot").
    name text PRIMARY KEY CHECK (name <> '' AND name = lower(name)),
    -- One of the brands mise ships, here from the start: a rebuilt database has it again, so removing one keeps its
    -- row (`removed`) for the curation export to write out. One added by hand is deleted when removed.
    seeded boolean NOT NULL DEFAULT false,
    -- A shipped brand curation took away (`unbrand`); matching skips it.
    removed boolean NOT NULL DEFAULT false,
    CHECK (seeded OR NOT removed)
);

INSERT INTO brands (name, seeded) VALUES
    ('bachan''s', true),
    ('beyond', true),
    ('beyond meat', true),
    ('blue diamond', true),
    ('bonne maman', true),
    ('boursin', true),
    ('cholula', true),
    ('cinnabon', true),
    ('colavita', true),
    ('craisins', true),
    ('eat smart', true),
    ('frank''s', true),
    ('frank''s redhot', true),
    ('honest turkey', true),
    ('honeysuckle white', true),
    ('jazz', true),
    ('kerrygold', true),
    ('kikkoman', true),
    ('mccormick', true),
    ('mccormick grill mates', true),
    ('mike''s hot honey', true),
    ('nature made', true),
    ('ocean spray', true),
    ('old bay', true),
    ('old el paso', true),
    ('open blue', true),
    ('oreo', true),
    ('philadelphia', true),
    ('pillsbury', true),
    ('redhot', true),
    ('reynolds kitchens', true),
    ('sabatino tartufi', true),
    ('sabra', true),
    ('sargento', true),
    ('should taste good', true),
    ('standard meat', true),
    ('stuffed puffs', true),
    ('sunkist', true),
    ('tillamook', true),
    ('tillamook country smoker', true),
    ('uncle ben''s', true),
    ('utz', true);
