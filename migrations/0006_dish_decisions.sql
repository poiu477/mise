-- Decisions about whether two recipe records belong in one dish, made by a person or by Claude on their behalf.
-- Keyed by source and external id rather than variant id, so they survive re-imports and a rebuilt database.
-- group-dishes honours them on every run: 'merge' joins the two dishes (after drift splitting, so it sticks);
-- 'keep_apart' stops the pair from being proposed again.
CREATE TABLE dish_decisions (
    a_source    text NOT NULL,
    a_external  text NOT NULL,
    b_source    text NOT NULL,
    b_external  text NOT NULL,
    decision    text NOT NULL CHECK (decision IN ('merge', 'keep_apart')),
    decided_by  text NOT NULL CHECK (decided_by IN ('user', 'claude')),
    note        text,
    PRIMARY KEY (a_source, a_external, b_source, b_external)
);
