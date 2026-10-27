-- Media ingest: where each stored file came from, and the web-size copies made from it.

-- The archive path (relative to the archive root, as variants and ingredient lines reference it) each stored
-- file was read from. Lets a re-run skip files it already stored without hashing them again, and lets variant
-- and food links be rebuilt from the references alone.
CREATE TABLE media_sources (
    path        text PRIMARY KEY,
    media_id    uuid NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    created_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX media_sources_media ON media_sources (media_id);

-- Resized copies for phones and kitchen displays: longest side at most max_side, never upscaled.
CREATE TABLE media_derivatives (
    media_id    uuid NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    max_side    int  NOT NULL,
    path        text NOT NULL,               -- relative to MEDIA_ROOT
    mime        text NOT NULL,
    bytes       bigint NOT NULL,
    width       int  NOT NULL,
    height      int  NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (media_id, max_side)
);

-- Set when an original could not be decoded, so derivative runs stop retrying it.
ALTER TABLE media ADD COLUMN decode_error text;

-- Step images: position orders every step image of a variant (by step, then by image within the step), so
-- one step can carry several images and the same image can appear under two steps.
COMMENT ON COLUMN variant_media.position IS
    'hero: 0; step: order among all of the variant''s step images (by step, then image within the step)';
CREATE INDEX variant_media_media ON variant_media (media_id);
CREATE INDEX variant_media_step ON variant_media (step_id) WHERE step_id IS NOT NULL;
