-- Add new columns to the `tasks` table, to store retry attempts and failure reasons.
ALTER TABLE tasks
    ADD COLUMN retry_attempt    INTEGER NULL,
    ADD COLUMN tags             TEXT[]  NOT NULL COLLATE case_insensitive DEFAULT '{}';
