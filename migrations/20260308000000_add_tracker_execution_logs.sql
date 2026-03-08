-- Table to store tracker execution logs with per-step timing phases.
CREATE TABLE IF NOT EXISTS tracker_execution_logs
(
    id                 UUID        PRIMARY KEY NOT NULL,
    tracker_id         UUID        NOT NULL REFERENCES trackers (id) ON DELETE CASCADE,
    job_id             UUID,
    started_at         TIMESTAMPTZ NOT NULL,
    finished_at        TIMESTAMPTZ NOT NULL,
    status             SMALLINT    NOT NULL,
    error              TEXT,
    is_manual          BOOL        NOT NULL DEFAULT FALSE,
    retry_attempt      SMALLINT,
    max_retry_attempts SMALLINT,
    revision_size      BIGINT,
    has_changes        BOOL,
    duration_ms        BIGINT    NOT NULL DEFAULT 0,
    phases             BYTEA
);

CREATE INDEX idx_tracker_execution_logs_tracker_started
    ON tracker_execution_logs (tracker_id, started_at DESC);

CREATE INDEX idx_tracker_execution_logs_started_at
    ON tracker_execution_logs (started_at);
