-- The key a device gives an item it adds to the list, so sending the same addition again (its answer was lost)
-- doesn't add it twice. Scripts can leave it out; Postgres never counts two missing keys as the same.

ALTER TABLE shopping_extras ADD COLUMN client_key text CHECK (length(client_key) BETWEEN 1 AND 100);
CREATE UNIQUE INDEX shopping_extras_client_key ON shopping_extras (household_id, client_key);
