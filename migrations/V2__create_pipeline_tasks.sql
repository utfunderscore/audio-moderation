CREATE TYPE pipeline_task_status AS ENUM (
    'PENDING',
    'STARTED_AUDIO_PROCESSING',
    'AUDIO_PROCESSING_FINISHED',
    'STARTED_ASR',
    'ASR_FINISHED',
    'STARTED_MODERATION_PROCESSING',
    'MODERATION_PROCESSING_FINISHED',
    'SUCCEEDED',
    'FAILED',
    'TIMED_OUT',
    'CANCELLED'
);

CREATE TABLE pipeline_tasks (
    task_id        SERIAL PRIMARY KEY,
    tenant_id      TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    caller_reference TEXT,
    status         pipeline_task_status NOT NULL DEFAULT 'PENDING',
    execution_arn  TEXT,
    asr_task_id    TEXT UNIQUE,
    dispatch_started_at TIMESTAMPTZ,
    attempt_count  INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    error_code     TEXT,
    error_message  TEXT,
    started_at     TIMESTAMPTZ,
    completed_at   TIMESTAMPTZ,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT pipeline_tasks_completion_time_valid
        CHECK (completed_at IS NULL OR started_at IS NULL OR completed_at >= started_at),
    CONSTRAINT pipeline_tasks_idempotency_unique
        UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX idx_pipeline_tasks_status
    ON pipeline_tasks (status);

CREATE TABLE pipeline_task_inputs (
    task_id       INTEGER NOT NULL REFERENCES pipeline_tasks (task_id) ON DELETE CASCADE,
    sequence      INTEGER NOT NULL CHECK (sequence >= 0),
    audio_s3_uri  TEXT NOT NULL,

    PRIMARY KEY (task_id, sequence)
);

CREATE TABLE audio_processing_outputs (
    task_id                 INTEGER PRIMARY KEY REFERENCES pipeline_tasks (task_id) ON DELETE CASCADE,
    stitched_audio_s3_uri   TEXT NOT NULL,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE asr_outputs (
    task_id     INTEGER PRIMARY KEY REFERENCES pipeline_tasks (task_id) ON DELETE CASCADE,
    transcript  TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE moderation_processing_outputs (
    task_id      INTEGER PRIMARY KEY REFERENCES pipeline_tasks (task_id) ON DELETE CASCADE,
    violence     FLOAT NOT NULL,
    sexual       FLOAT NOT NULL,
    hate_speech  FLOAT NOT NULL,
    harassment   FLOAT NOT NULL,
    self_harm    FLOAT NOT NULL,
    profanity    FLOAT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
