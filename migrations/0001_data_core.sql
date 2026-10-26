-- Phase 1 data core: sources, dishes and variants, ingredient lines, the food tree, evidence and review.

CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- Where recipes come from. A source is anything that supplies variants: a seed bundle, a website, a file import, a person.
CREATE TABLE sources (
    id          uuid PRIMARY KEY,
    key         text NOT NULL UNIQUE,          -- stable machine key, e.g. 'seed:hellofresh-us', 'web', 'user'
    name        text NOT NULL,
    kind        text NOT NULL CHECK (kind IN ('bundle', 'web', 'file', 'user')),
    created_at  timestamptz NOT NULL DEFAULT now()
);

-- Raw records kept verbatim so imports can be re-processed without re-fetching. A re-import whose content
-- changed adds a new record (and a new variant) rather than overwriting the old one.
CREATE TABLE source_records (
    id           uuid PRIMARY KEY,
    source_id    uuid NOT NULL REFERENCES sources(id),
    external_id  text NOT NULL,
    url          text,
    raw          jsonb NOT NULL,
    content_hash text NOT NULL,                 -- sha256 of the normalized record
    imported_at  timestamptz NOT NULL DEFAULT now(),
    UNIQUE (source_id, external_id, content_hash)
);
CREATE INDEX source_records_external ON source_records (source_id, external_id, imported_at DESC);

CREATE TYPE content_type AS ENUM ('recipe', 'ready_meal', 'add_on', 'product');

-- The thing people choose. Groups every version of a recipe, whatever its source.
CREATE TABLE dishes (
    id                 uuid PRIMARY KEY,
    title              text NOT NULL,
    subtitle           text,
    slug               text NOT NULL,
    content_type       content_type NOT NULL DEFAULT 'recipe',
    default_variant_id uuid,                     -- set after variants exist; FK added below
    created_at         timestamptz NOT NULL DEFAULT now(),
    updated_at         timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX dishes_title_trgm ON dishes USING gin (title gin_trgm_ops);

-- One version of a dish: a published issue, an imported page, or our own edit or fork.
CREATE TABLE variants (
    id                 uuid PRIMARY KEY,
    dish_id            uuid REFERENCES dishes(id),        -- null until grouping runs
    source_record_id   uuid REFERENCES source_records(id), -- null for versions written in the app
    origin             text NOT NULL CHECK (origin IN ('source', 'user')),
    parent_variant_id  uuid REFERENCES variants(id),      -- lineage: the version this one was copied or forked from
    title              text NOT NULL,
    subtitle           text,
    description        text,
    published_at       timestamptz,
    has_steps          boolean NOT NULL,
    meta               jsonb NOT NULL DEFAULT '{}',       -- cuisines, tags, difficulty, times, nutrition
    content_hash       text NOT NULL,                     -- of ingredients + amounts + steps, for spotting identical versions
    created_at         timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX variants_dish ON variants (dish_id);
CREATE INDEX variants_parent ON variants (parent_variant_id);
CREATE UNIQUE INDEX variants_source_record ON variants (source_record_id) WHERE source_record_id IS NOT NULL;

ALTER TABLE dishes ADD CONSTRAINT dishes_default_variant_fk
    FOREIGN KEY (default_variant_id) REFERENCES variants(id) DEFERRABLE INITIALLY DEFERRED;

-- Serving counts a variant publishes amounts for. Anything else is scaled.
CREATE TABLE variant_yields (
    variant_id uuid NOT NULL REFERENCES variants(id) ON DELETE CASCADE,
    servings   numeric NOT NULL CHECK (servings > 0),
    PRIMARY KEY (variant_id, servings)
);

-- The food tree. A child is a kind of its parent that can stand in for it in most recipes.
CREATE TYPE food_kind AS ENUM ('category', 'food', 'brand', 'variety', 'cut', 'form', 'proprietary');
CREATE TYPE review_status AS ENUM ('proposed', 'reviewed', 'tested', 'rejected');

CREATE TABLE foods (
    id          uuid PRIMARY KEY,
    name        text NOT NULL,
    slug        text NOT NULL UNIQUE,
    parent_id   uuid REFERENCES foods(id),
    kind        food_kind NOT NULL DEFAULT 'food',
    status      review_status NOT NULL DEFAULT 'proposed',
    props       jsonb NOT NULL DEFAULT '{}',   -- store section, pantry staple, density, default unit; inherited by children
    created_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX foods_parent ON foods (parent_id);
CREATE INDEX foods_name_trgm ON foods USING gin (name gin_trgm_ops);

CREATE TABLE food_aliases (
    food_id  uuid NOT NULL REFERENCES foods(id) ON DELETE CASCADE,
    alias    text NOT NULL,                  -- normalized: lowercase, folded, no punctuation
    PRIMARY KEY (alias, food_id)
);
CREATE INDEX food_aliases_trgm ON food_aliases USING gin (alias gin_trgm_ops);

-- Links between foods that are not is-a.
CREATE TYPE food_relation_kind AS ENUM ('related', 'substitute', 'made_from');
CREATE TABLE food_relations (
    id         uuid PRIMARY KEY,
    from_food  uuid NOT NULL REFERENCES foods(id) ON DELETE CASCADE,
    to_food    uuid NOT NULL REFERENCES foods(id) ON DELETE CASCADE,
    kind       food_relation_kind NOT NULL,
    detail     jsonb NOT NULL DEFAULT '{}',  -- ratio and note for substitutes; amount for made_from components
    status     review_status NOT NULL DEFAULT 'proposed',
    CHECK (from_food <> to_food)
);
CREATE INDEX food_relations_from ON food_relations (from_food, kind);

-- Facts about a food, each with the evidence behind it. A food can hold several facts of one kind from different tiers.
CREATE TYPE evidence_tier AS ENUM ('label', 'brand_support', 'regional_label', 'inferred', 'copycat');
CREATE TABLE food_facts (
    id           uuid PRIMARY KEY,
    food_id      uuid NOT NULL REFERENCES foods(id) ON DELETE CASCADE,
    kind         text NOT NULL CHECK (kind IN ('ingredients', 'allergens', 'homemade', 'buy', 'description', 'packet_size')),
    value        jsonb NOT NULL,
    tier         evidence_tier NOT NULL,
    source_ref   text,                       -- a URL, a chat date, a region and product name
    observed_at  date,
    status       review_status NOT NULL DEFAULT 'proposed',
    created_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX food_facts_food ON food_facts (food_id, kind);

-- One ingredient line of a variant, pointing at the most specific food it names.
CREATE TABLE ingredient_lines (
    id          uuid PRIMARY KEY,
    variant_id  uuid NOT NULL REFERENCES variants(id) ON DELETE CASCADE,
    line_key    text NOT NULL,               -- stable within the variant; steps reference it
    position    int  NOT NULL,
    raw_name    text NOT NULL,               -- as the source wrote it
    food_id     uuid REFERENCES foods(id),   -- null until matched
    note        text,
    shipped     boolean,                     -- supplied by the source (meal kit) vs. from the pantry
    allergens   text[] NOT NULL DEFAULT '{}',
    source_food jsonb,                       -- the source's own reference for this ingredient (id, image), for matching
    flexibility text CHECK (flexibility IN ('flexible', 'strict')),  -- null = the variant's default
    UNIQUE (variant_id, line_key)
);
CREATE INDEX ingredient_lines_food ON ingredient_lines (food_id);

-- Amounts per published yield.
CREATE TABLE line_amounts (
    line_id   uuid NOT NULL REFERENCES ingredient_lines(id) ON DELETE CASCADE,
    servings  numeric NOT NULL,
    amount    numeric,
    unit      text,
    PRIMARY KEY (line_id, servings)
);

CREATE TABLE steps (
    id          uuid PRIMARY KEY,
    variant_id  uuid NOT NULL REFERENCES variants(id) ON DELETE CASCADE,
    position    int  NOT NULL,
    text        text NOT NULL,               -- as published
    template    text,                        -- with amount references, once parsed
    UNIQUE (variant_id, position)
);

-- Media stored once by content hash; variants and steps point at it.
CREATE TABLE media (
    id          uuid PRIMARY KEY,
    sha256      text NOT NULL UNIQUE,
    mime        text NOT NULL,
    bytes       bigint NOT NULL,
    width       int,
    height      int,
    path        text NOT NULL,               -- relative to MEDIA_ROOT
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE variant_media (
    variant_id  uuid NOT NULL REFERENCES variants(id) ON DELETE CASCADE,
    media_id    uuid NOT NULL REFERENCES media(id),
    role        text NOT NULL CHECK (role IN ('hero', 'step')),
    step_id     uuid REFERENCES steps(id) ON DELETE CASCADE,
    position    int NOT NULL DEFAULT 0,
    PRIMARY KEY (variant_id, media_id, role, position)
);

-- Anything a person should confirm: doubtful dish merges, new foods, proposed tree links, conflicting evidence.
CREATE TABLE review_items (
    id          uuid PRIMARY KEY,
    kind        text NOT NULL,               -- 'dish_merge', 'food_match', 'food_parent', 'evidence_conflict'
    subject     jsonb NOT NULL,
    proposal    jsonb NOT NULL,
    confidence  real,
    status      text NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'accepted', 'rejected', 'edited')),
    decision    jsonb,
    created_at  timestamptz NOT NULL DEFAULT now(),
    decided_at  timestamptz
);
CREATE INDEX review_items_open ON review_items (kind, status);
