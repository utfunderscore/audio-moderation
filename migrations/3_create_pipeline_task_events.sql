CREATE TABLE pipeline_task_events (
    event_id    BIGSERIAL PRIMARY KEY,
    task_id     INTEGER NOT NULL REFERENCES pipeline_tasks (task_id) ON DELETE CASCADE,
    event_name  TEXT NOT NULL CHECK (event_name <> ''),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (task_id, event_name)
);

CREATE INDEX idx_pipeline_task_events_task_id_event_id
    ON pipeline_task_events (task_id, event_id);
