-- Switch trackers_data.data to EXTERNAL storage since the application now handles
-- compression (gzip) before writing. EXTERNAL tells PostgreSQL to store the value
-- out-of-line (TOAST) when large but skip its own pglz compression pass, avoiding
-- double-compression overhead.
ALTER TABLE trackers_data ALTER COLUMN data SET STORAGE EXTERNAL;
