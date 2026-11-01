-- How often a household has had a version. group-dishes opens a dish on the version had most and names the dish after
-- it. Keyed by source and external id rather than variant id, so it survives re-imports and a rebuilt database.
CREATE TABLE preferred_versions (
    source      text NOT NULL,
    external_id text NOT NULL,
    times       integer NOT NULL CHECK (times > 0),
    PRIMARY KEY (source, external_id)
);
