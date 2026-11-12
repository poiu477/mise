-- Kitchen devices: a screen in the kitchen (an Echo Show, say) whose browser can't sign in comfortably or send
-- headers. A household admin issues a token with a name; it starts as a one-time link, and opening the link on the
-- device trades it for a long-lived cookie. A kitchen session reads the household's plan and recipes, nothing else.
-- Both secrets are random 256-bit values stored only as SHA-256 hashes, like sessions and API tokens.
CREATE TABLE kitchen_tokens (
    id               uuid PRIMARY KEY,
    household_id     uuid NOT NULL REFERENCES households(id) ON DELETE CASCADE,
    -- Who issued it, for the list; the token outlives the account that made it.
    created_by       uuid REFERENCES users(id) ON DELETE SET NULL,
    name             text NOT NULL,
    -- The one-time link, until it is opened; an unopened link stays, refused, once it expires.
    link_hash        text UNIQUE,
    link_expires_at  timestamptz,
    -- The device's cookie, from when the link is opened.
    token_hash       text UNIQUE,
    created_at       timestamptz NOT NULL DEFAULT now(),
    linked_at        timestamptz,
    last_used_at     timestamptz,
    CHECK ((link_hash IS NULL) = (link_expires_at IS NULL)),
    -- Waiting for its link to be opened, or linked to a device: one or the other.
    CHECK ((link_hash IS NULL) <> (token_hash IS NULL)),
    CHECK ((token_hash IS NULL) = (linked_at IS NULL))
);
CREATE INDEX kitchen_tokens_household ON kitchen_tokens (household_id);
