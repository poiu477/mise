-- Records a household imports itself (a web page, later a file) are that household's own, like the versions made
-- from them: two households that import the same page each keep their own record of it, and neither can tell the
-- other has it. Bundle records belong to no household, as before.
ALTER TABLE source_records ADD COLUMN household_id uuid REFERENCES households(id);

-- One record per source, id, content and owner. Records without an owner still count as the same owner, so a
-- bundle's record can't be stored twice.
ALTER TABLE source_records DROP CONSTRAINT source_records_source_id_external_id_content_hash_key;
CREATE UNIQUE INDEX source_records_identity
    ON source_records (source_id, external_id, content_hash, household_id) NULLS NOT DISTINCT;
