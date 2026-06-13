-- Move text columns that need search/filtering from the nondeterministic
-- case-insensitive collation to a deterministic collation. Case-insensitive
-- behavior is handled explicitly by API normalization (tags) and ILIKE (names).
DROP INDEX IF EXISTS trackers_tags_idx;

ALTER TABLE trackers
    ALTER COLUMN name TYPE TEXT COLLATE "C",
    ALTER COLUMN tags TYPE TEXT[] COLLATE "C";

ALTER TABLE tasks
    ALTER COLUMN tags TYPE TEXT[] COLLATE "C";

CREATE INDEX trackers_tags_idx ON trackers USING gin(tags);
