use chrono::{DateTime, Utc};
use sqlx::PgPool;

/// Errors returned while accessing persisted task WebSocket subscriptions.
#[derive(Debug, thiserror::Error)]
pub enum PipelineTaskWebSocketConnectionError {
    #[error("failed to create or retrieve pipeline task WebSocket connection")]
    CreateOrGet(#[source] sqlx::Error),

    #[error("failed to retrieve pipeline task WebSocket connections")]
    List(#[source] sqlx::Error),

    #[error("failed to remove pipeline task WebSocket connections")]
    Remove(#[source] sqlx::Error),

    #[error("WebSocket connection is already subscribed to pipeline task {task_id}")]
    AlreadySubscribed { task_id: i32 },
}

/// A WebSocket connection subscribed to a pipeline task's event stream.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct PipelineTaskWebSocketConnection {
    pub task_id: i32,
    pub connection_id: String,
    pub created_at: DateTime<Utc>,
    /// Whether this call created the subscription rather than replaying it.
    pub created: bool,
}

/// Values required to register a WebSocket connection for a pipeline task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPipelineTaskWebSocketConnection<'a> {
    pub task_id: i32,
    pub connection_id: &'a str,
}

/// SQLx-backed access to the `pipeline_task_websocket_connections` table.
#[derive(Clone)]
pub struct PipelineTaskWebSocketConnectionStore {
    pool: PgPool,
}

impl PipelineTaskWebSocketConnectionStore {
    /// Creates a store using the application's PostgreSQL connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Registers a connection for a task, or returns the existing subscription.
    pub async fn create_or_get(
        &self,
        connection: NewPipelineTaskWebSocketConnection<'_>,
    ) -> Result<PipelineTaskWebSocketConnection, PipelineTaskWebSocketConnectionError> {
        let created = sqlx::query_as::<_, PipelineTaskWebSocketConnection>(
            r#"
                INSERT INTO pipeline_task_websocket_connections (task_id, connection_id)
                VALUES ($1, $2)
                ON CONFLICT (connection_id) DO NOTHING
                RETURNING task_id, connection_id, created_at, TRUE AS created
            "#,
        )
        .bind(connection.task_id)
        .bind(connection.connection_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(PipelineTaskWebSocketConnectionError::CreateOrGet)?;
        if let Some(created) = created {
            return Ok(created);
        }
        let existing = sqlx::query_as::<_, PipelineTaskWebSocketConnection>(
            r#"
                SELECT task_id, connection_id, created_at, FALSE AS created
                FROM pipeline_task_websocket_connections WHERE connection_id = $1
            "#,
        )
        .bind(connection.connection_id)
        .fetch_one(&self.pool)
        .await
        .map_err(PipelineTaskWebSocketConnectionError::CreateOrGet)?;
        if existing.task_id == connection.task_id {
            Ok(existing)
        } else {
            Err(PipelineTaskWebSocketConnectionError::AlreadySubscribed {
                task_id: existing.task_id,
            })
        }
    }

    /// Returns every connection subscribed to a task.
    pub async fn list(
        &self,
        task_id: i32,
    ) -> Result<Vec<PipelineTaskWebSocketConnection>, PipelineTaskWebSocketConnectionError> {
        sqlx::query_as::<_, PipelineTaskWebSocketConnection>(
            r#"
                SELECT task_id, connection_id, created_at, TRUE AS created
                FROM pipeline_task_websocket_connections
                WHERE task_id = $1
                ORDER BY connection_id
            "#,
        )
        .bind(task_id)
        .fetch_all(&self.pool)
        .await
        .map_err(PipelineTaskWebSocketConnectionError::List)
    }

    /// Removes all subscriptions for a disconnected WebSocket connection.
    pub async fn remove(
        &self,
        connection_id: &str,
    ) -> Result<u64, PipelineTaskWebSocketConnectionError> {
        sqlx::query("DELETE FROM pipeline_task_websocket_connections WHERE connection_id = $1")
            .bind(connection_id)
            .execute(&self.pool)
            .await
            .map(|result| result.rows_affected())
            .map_err(PipelineTaskWebSocketConnectionError::Remove)
    }
}
