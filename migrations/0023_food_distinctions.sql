-- Pairs of food names a person looked at and said are different foods, so the review queue stops offering them as
-- near-duplicates (T37). Kept by name, not by food id: `match-foods --reset` deletes the foods matching made, and a
-- database rebuilt from the same recipes makes them afresh with new ids, so ids would lose the answer. This is how
-- dish decisions keep a pair "kept apart" (by source record, not row id).
CREATE TABLE food_distinctions (
    -- Both names as `food_names::duplicate_key` writes them: every word folded to its singular, the words that only
    -- say how the food was prepared dropped, the rest sorted. So the answer holds however the pair is spelled next
    -- time ("Red Onion, Diced" and "Diced Red Onions" are one key).
    a_name text NOT NULL CHECK (a_name <> ''),
    b_name text NOT NULL CHECK (b_name <> ''),
    -- Who said so: "user", or "claude" when decided on their behalf.
    decided_by text NOT NULL DEFAULT 'user',
    -- Why, in a few words: "different cuts".
    note text,
    decided_at timestamptz NOT NULL DEFAULT now(),
    -- One row per pair whichever way round it was answered.
    CHECK (a_name < b_name),
    PRIMARY KEY (a_name, b_name)
);
