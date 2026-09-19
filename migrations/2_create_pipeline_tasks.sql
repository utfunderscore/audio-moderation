CREATE TYPE pipeline_task_outcome AS ENUM (
    'SUCCEEDED',
    'FAILED',
    'TIMED_OUT',
    'CANCELLED'
);

CREATE TYPE pipeline_step_status AS ENUM (
    'PENDING',
    'PROCESSING',
    'COMPLETED',
    'FAILED'
);

CREATE TYPE pipeline_callback_step AS ENUM (
    'TRANSCRIPTION',
    'MODERATION'
);

CREATE TABLE pipeline_tasks (
    task_id        SERIAL PRIMARY KEY,
    evaluation_id  UUID NOT NULL DEFAULT gen_random_uuid(),
    tenant_id      TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    caller_reference TEXT,
    outcome        pipeline_task_outcome,
    execution_arn  TEXT,
    dispatch_started_at TIMESTAMPTZ,
    attempt_count  INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    completed_at   TIMESTAMPTZ,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT pipeline_tasks_outcome_completion_consistent
        CHECK ((outcome IS NULL) = (completed_at IS NULL)),
    CONSTRAINT pipeline_tasks_evaluation_id_unique
        UNIQUE (evaluation_id),
    CONSTRAINT pipeline_tasks_idempotency_unique
        UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX idx_pipeline_tasks_outcome
    ON pipeline_tasks (outcome);

CREATE TABLE pipeline_task_inputs (
    task_id       INTEGER NOT NULL REFERENCES pipeline_tasks (task_id) ON DELETE CASCADE,
    sequence      INTEGER NOT NULL CHECK (sequence >= 0),
    audio_s3_uri  TEXT NOT NULL,

    PRIMARY KEY (task_id, sequence)
);

CREATE TABLE audio_processing_tasks (
    task_id                 INTEGER PRIMARY KEY REFERENCES pipeline_tasks (task_id) ON DELETE CASCADE,
    status                  pipeline_step_status NOT NULL DEFAULT 'PENDING',
    stitched_audio_s3_uri   TEXT,
    error_code              TEXT,
    error_message           TEXT,
    started_at              TIMESTAMPTZ,
    completed_at            TIMESTAMPTZ,
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT audio_processing_tasks_completion_time_valid
        CHECK (completed_at IS NULL OR started_at IS NULL OR completed_at >= started_at),
    CONSTRAINT audio_processing_tasks_completed_output_present
        CHECK (status != 'COMPLETED' OR stitched_audio_s3_uri IS NOT NULL)
);

CREATE TABLE transcription_tasks (
    task_id           INTEGER PRIMARY KEY REFERENCES pipeline_tasks (task_id) ON DELETE CASCADE,
    status            pipeline_step_status NOT NULL DEFAULT 'PENDING',
    external_task_id  TEXT UNIQUE,
    transcript        TEXT,
    error_code        TEXT,
    error_message     TEXT,
    started_at        TIMESTAMPTZ,
    completed_at      TIMESTAMPTZ,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT transcription_tasks_completion_time_valid
        CHECK (completed_at IS NULL OR started_at IS NULL OR completed_at >= started_at),
    CONSTRAINT transcription_tasks_completed_output_present
        CHECK (status != 'COMPLETED' OR transcript IS NOT NULL)
);

-- Task tokens are bearer credentials. Store only their SHA-256 digests and
-- retain every token issued for a callback-capable pipeline step so callbacks
-- can be matched to the exact workflow attempt even when a Lambda retries.
CREATE TABLE pipeline_callback_attempts (
    task_id             INTEGER NOT NULL REFERENCES pipeline_tasks (task_id) ON DELETE CASCADE,
    step                pipeline_callback_step NOT NULL,
    task_token_hash     CHAR(64) NOT NULL UNIQUE,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    PRIMARY KEY (task_id, step, task_token_hash)
);

CREATE TABLE moderation_tasks (
    task_id                 INTEGER PRIMARY KEY REFERENCES pipeline_tasks (task_id) ON DELETE CASCADE,
    status                  pipeline_step_status NOT NULL DEFAULT 'PENDING',
    external_task_id        TEXT UNIQUE,
    sexual                  DOUBLE PRECISION,
    hate_or_discrimination  DOUBLE PRECISION,
    harassment_or_abuse     DOUBLE PRECISION,
    violence_or_threats     DOUBLE PRECISION,
    asking_for_pii          DOUBLE PRECISION,
    error_code              TEXT,
    error_message           TEXT,
    started_at              TIMESTAMPTZ,
    completed_at            TIMESTAMPTZ,
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT moderation_tasks_completion_time_valid
        CHECK (completed_at IS NULL OR started_at IS NULL OR completed_at >= started_at),
    CONSTRAINT moderation_tasks_completed_output_present
        CHECK (
            status != 'COMPLETED'
            OR (
                sexual IS NOT NULL
                AND hate_or_discrimination IS NOT NULL
                AND harassment_or_abuse IS NOT NULL
                AND violence_or_threats IS NOT NULL
                AND asking_for_pii IS NOT NULL
            )
        ),
    CONSTRAINT moderation_tasks_scores_in_range
        CHECK (
            (sexual IS NULL OR sexual >= 0.0 AND sexual <= 1.0)
            AND (hate_or_discrimination IS NULL OR hate_or_discrimination >= 0.0 AND hate_or_discrimination <= 1.0)
            AND (harassment_or_abuse IS NULL OR harassment_or_abuse >= 0.0 AND harassment_or_abuse <= 1.0)
            AND (violence_or_threats IS NULL OR violence_or_threats >= 0.0 AND violence_or_threats <= 1.0)
            AND (asking_for_pii IS NULL OR asking_for_pii >= 0.0 AND asking_for_pii <= 1.0)
        )
);
