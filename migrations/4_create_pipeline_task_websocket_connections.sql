CREATE TABLE pipeline_task_websocket_connections (
    task_id        INTEGER NOT NULL REFERENCES pipeline_tasks (task_id) ON DELETE CASCADE,
    connection_id  TEXT NOT NULL CHECK (connection_id <> ''),
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    PRIMARY KEY (task_id, connection_id)
);

CREATE INDEX idx_pipeline_task_websocket_connections_connection_id
    ON pipeline_task_websocket_connections (connection_id);
