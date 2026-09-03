CREATE TYPE review_job_status AS ENUM (
    'AWAITING_UPLOAD',
    'QUEUED',
    'STARTED_PREPROCESSING_AUDIO',
    'FINISHED_PREPROCESSING_AUDIO',
    'STARTED_TRANSCRIBING',
    'FINISHED_TRANSCRIBING',
    'STARTED_EVALUATING',
    'FINISHED_EVALUATING',
    'STARTED_PERSISTING_RESULT',
    'COMPLETED',
    'FAILED'
);

CREATE TABLE review_jobs (
    job_id          UUID PRIMARY KEY,
    tenant_id       TEXT NOT NULL,
    idempotency_key TEXT,
    status          review_job_status NOT NULL DEFAULT 'AWAITING_UPLOAD',
    input_file_path TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT review_jobs_input_file_path_unique
        UNIQUE (input_file_path),
    CONSTRAINT review_jobs_idempotency_unique
        UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX idx_review_jobs_tenant_created
    ON review_jobs (tenant_id, created_at DESC);
