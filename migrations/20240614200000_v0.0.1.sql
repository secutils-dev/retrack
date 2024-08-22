-- Create collation for case-insensitive sorting.
CREATE COLLATION IF NOT EXISTS case_insensitive (provider = icu, locale = 'und-u-ks-level2', deterministic = false);

-- Table to store trackers.
CREATE TABLE IF NOT EXISTS trackers
(
    id          UUID PRIMARY KEY NOT NULL,
    name        TEXT             NOT NULL UNIQUE COLLATE case_insensitive,
    url         TEXT             NOT NULL COLLATE case_insensitive,
    target      BYTEA            NOT NULL,
    config      BYTEA            NOT NULL,
    tags        TEXT[]           NOT NULL COLLATE case_insensitive DEFAULT '{}',
    created_at  TIMESTAMPTZ      NOT NULL,
    updated_at  TIMESTAMPTZ      NOT NULL,
    -- Internal fields for tracking job status.
    job_needed  BOOL             NOT NULL,
    job_id      UUID UNIQUE
);

-- Table to store trackers captured data.
CREATE TABLE IF NOT EXISTS trackers_data
(
    id         UUID PRIMARY KEY NOT NULL,
    data       BYTEA            NOT NULL,
    created_at TIMESTAMPTZ      NOT NULL,
    tracker_id UUID             NOT NULL REFERENCES trackers (id) ON DELETE CASCADE,
    UNIQUE (created_at, tracker_id)
);

-- Table to store notifications.
CREATE TABLE IF NOT EXISTS notifications
(
    id           SERIAL PRIMARY KEY NOT NULL,
    destination  BYTEA              NOT NULL,
    content      BYTEA              NOT NULL,
    scheduled_at TIMESTAMPTZ        NOT NULL
);

-- Table to store scheduler jobs.
CREATE TABLE IF NOT EXISTS scheduler_jobs
(
    id                  UUID PRIMARY KEY NOT NULL,
    last_updated        BIGINT,
    next_tick           BIGINT,
    last_tick           BIGINT,
    job_type            INTEGER          NOT NULL,
    count               INTEGER,
    ran                 BOOLEAN,
    stopped             BOOLEAN,
    schedule            TEXT,
    repeating           BOOLEAN,
    repeated_every      BIGINT,
    time_offset_seconds INTEGER,
    extra               BYTEA
);

-- Table to store scheduler job notifications.
CREATE TABLE IF NOT EXISTS scheduler_notifications
(
    id     UUID PRIMARY KEY NOT NULL,
    job_id UUID,
    extra  BYTEA
);

-- Table to store scheduler job notification states.
CREATE TABLE IF NOT EXISTS scheduler_notification_states
(
    id    UUID    NOT NULL REFERENCES scheduler_notifications (id) ON DELETE CASCADE,
    state INTEGER NOT NULL,
    PRIMARY KEY (id, state)
);
