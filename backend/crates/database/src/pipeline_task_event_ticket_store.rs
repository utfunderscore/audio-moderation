use chrono::{DateTime, Duration, Utc};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

#[derive(Debug, thiserror::Error)]
pub enum PipelineTaskEventTicketError {
    #[error("failed to create pipeline task-event ticket")]
    Create(#[source] sqlx::Error),

    #[error("failed to consume pipeline task-event ticket")]
    Consume(#[source] sqlx::Error),

    #[error("task-event ticket is invalid or expired")]
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineTaskEventTicket {
    pub task_id: i32,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct PipelineTaskEventTicketStore {
    pool: PgPool,
}

impl PipelineTaskEventTicketStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn create(
        &self,
        task_id: i32,
        ticket: &str,
        lifetime: Duration,
    ) -> Result<PipelineTaskEventTicket, PipelineTaskEventTicketError> {
        let expires_at = Utc::now() + lifetime;
        sqlx::query_as::<_, (i32, DateTime<Utc>)>(
            r#"
                WITH expired AS (
                    DELETE FROM pipeline_task_event_tickets WHERE expires_at <= NOW()
                )
                INSERT INTO pipeline_task_event_tickets (ticket_hash, task_id, expires_at)
                VALUES ($1, $2, $3)
                RETURNING task_id, expires_at
            "#,
        )
        .bind(token_hash(ticket))
        .bind(task_id)
        .bind(expires_at)
        .fetch_one(&self.pool)
        .await
        .map(|(task_id, expires_at)| PipelineTaskEventTicket {
            task_id,
            expires_at,
        })
        .map_err(PipelineTaskEventTicketError::Create)
    }

    pub async fn consume(
        &self,
        ticket: &str,
    ) -> Result<PipelineTaskEventTicket, PipelineTaskEventTicketError> {
        sqlx::query_as::<_, (i32, DateTime<Utc>)>(
            r#"
                DELETE FROM pipeline_task_event_tickets
                WHERE ticket_hash = $1 AND expires_at > NOW()
                RETURNING task_id, expires_at
            "#,
        )
        .bind(token_hash(ticket))
        .fetch_optional(&self.pool)
        .await
        .map_err(PipelineTaskEventTicketError::Consume)?
        .map(|(task_id, expires_at)| PipelineTaskEventTicket {
            task_id,
            expires_at,
        })
        .ok_or(PipelineTaskEventTicketError::Invalid)
    }
}

fn token_hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}
