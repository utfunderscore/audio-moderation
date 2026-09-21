CREATE TABLE pipeline_task_websocket_connections (
    task_id        INTEGER NOT NULL REFERENCES pipeline_tasks (task_id) ON DELETE CASCADE,
    connection_id  TEXT NOT NULL CHECK (connection_id <> ''),
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    PRIMARY KEY (task_id, connection_id),
    -- A connection is deliberately limited to one task stream. A client that
    -- needs a second review stream must establish a second WebSocket.
    CONSTRAINT pipeline_task_websocket_connections_connection_unique
        UNIQUE (connection_id)
);

CREATE INDEX idx_pipeline_task_websocket_connections_connection_id
    ON pipeline_task_websocket_connections (connection_id);

CREATE TABLE pipeline_task_event_tickets (
    ticket_hash CHAR(64) PRIMARY KEY,
    task_id     INTEGER NOT NULL REFERENCES pipeline_tasks (task_id) ON DELETE CASCADE,
    expires_at  TIMESTAMPTZ NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT pipeline_task_event_tickets_expiry_valid
        CHECK (expires_at > created_at)
);

CREATE INDEX idx_pipeline_task_event_tickets_expiry
    ON pipeline_task_event_tickets (expires_at);
