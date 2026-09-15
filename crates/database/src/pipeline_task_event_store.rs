use chrono::{DateTime, Utc};
use sqlx::PgPool;

/// Errors returned while accessing persisted pipeline task events.
#[derive(Debug, thiserror::Error)]
pub enum PipelineTaskEventError {
    #[error("failed to create pipeline task event")]
    Create(#[source] sqlx::Error),

    #[error("failed to retrieve pipeline task events")]
    List(#[source] sqlx::Error),
}

/// A durable event emitted during a pipeline task's lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct PipelineTaskEvent {
    pub event_id: i64,
    pub task_id: i32,
    pub event_name: String,
    pub created_at: DateTime<Utc>,
}

/// Values required to record a pipeline task event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPipelineTaskEvent<'a> {
    pub task_id: i32,
    pub event_name: &'a str,
}

/// SQLx-backed access to the `pipeline_task_events` table.
#[derive(Clone)]
pub struct PipelineTaskEventStore {
    pool: PgPool,
}

impl PipelineTaskEventStore {
    /// Creates a store using the application's PostgreSQL connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Records an event for a pipeline task and returns its durable replay position.
    pub async fn create(
        &self,
        event: NewPipelineTaskEvent<'_>,
    ) -> Result<PipelineTaskEvent, PipelineTaskEventError> {
        sqlx::query_as::<_, PipelineTaskEvent>(
            r#"
                INSERT INTO pipeline_task_events (task_id, event_name)
                VALUES ($1, $2)
                RETURNING event_id, task_id, event_name, created_at
            "#,
        )
        .bind(event.task_id)
        .bind(event.event_name)
        .fetch_one(&self.pool)
        .await
        .map_err(PipelineTaskEventError::Create)
    }

    /// Returns a task's events in the order they must be replayed to subscribers.
    pub async fn list(
        &self,
        task_id: i32,
    ) -> Result<Vec<PipelineTaskEvent>, PipelineTaskEventError> {
        sqlx::query_as::<_, PipelineTaskEvent>(
            r#"
                SELECT event_id, task_id, event_name, created_at
                FROM pipeline_task_events
                WHERE task_id = $1
                ORDER BY event_id
            "#,
        )
        .bind(task_id)
        .fetch_all(&self.pool)
        .await
        .map_err(PipelineTaskEventError::List)
    }
}
