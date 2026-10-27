-- Accounts. A household is the sharing boundary for plans, favourites and lists; each user belongs to one.
-- Sign-in is a local password, an OIDC identity, or both; browsers hold a session, scripts an API token.
CREATE TABLE households (
    id          uuid PRIMARY KEY,
    name        text NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE users (
    id             uuid PRIMARY KEY,
    username       text NOT NULL,
    display_name   text NOT NULL,
    email          text,
    password_hash  text,                        -- argon2id PHC string; null for single-sign-on-only accounts
    is_admin       boolean NOT NULL DEFAULT false,
    disabled       boolean NOT NULL DEFAULT false,
    household_id   uuid NOT NULL REFERENCES households(id),
    created_at     timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX users_username ON users (lower(username));
CREATE UNIQUE INDEX users_email ON users (lower(email)) WHERE email IS NOT NULL;

-- OIDC identities linked to a user (issuer + subject is the provider's stable id for a person).
CREATE TABLE user_identities (
    issuer      text NOT NULL,
    subject     text NOT NULL,
    user_id     uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (issuer, subject)
);

-- Browser sessions. Only a hash of the cookie value is stored.
CREATE TABLE sessions (
    token_hash    text PRIMARY KEY,
    user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at    timestamptz NOT NULL DEFAULT now(),
    last_seen_at  timestamptz NOT NULL DEFAULT now(),
    expires_at    timestamptz NOT NULL,
    user_agent    text
);
CREATE INDEX sessions_user ON sessions (user_id);

-- Personal API tokens for scripts and integrations; shown once, stored as a hash.
CREATE TABLE api_tokens (
    id            uuid PRIMARY KEY,
    user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name          text NOT NULL,
    token_hash    text NOT NULL UNIQUE,
    scope         text NOT NULL CHECK (scope IN ('read', 'read_write')),
    created_at    timestamptz NOT NULL DEFAULT now(),
    last_used_at  timestamptz
);

-- A single-sign-on attempt between the redirect to the provider and its callback (minutes).
CREATE TABLE oidc_logins (
    state_hash     text PRIMARY KEY,
    nonce          text NOT NULL,
    pkce_verifier  text NOT NULL,
    return_to      text NOT NULL,
    created_at     timestamptz NOT NULL DEFAULT now()
);
