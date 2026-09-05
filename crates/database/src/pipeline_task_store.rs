use chrono::{DateTime, Utc};
use sqlx::PgPool;

/// Errors returned while accessing or dispatching persisted pipeline tasks.
#[derive(Debug, thiserror::Error)]
pub enum PipelineTaskError {
    #[error("failed to create or retrieve pipeline task")]
    CreateOrGet(#[source] sqlx::Error),

    #[error("failed to record pipeline task execution")]
    RecordExecution(#[source] sqlx::Error),

    #[error("failed to claim pipeline task dispatch")]
    ClaimDispatch(#[source] sqlx::Error),

    #[error("failed to record pipeline task dispatch failure")]
    RecordDispatchFailure(#[source] sqlx::Error),
}

/// A pipeline task together with its ordered audio inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineTask {
    pub task_id: i32,
    pub tenant_id: String,
    pub idempotency_key: String,
    pub caller_reference: Option<String>,
    pub status: PipelineTaskStatus,
    pub execution_arn: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub audio_s3_uris: Vec<String>,
    /// Whether this call created the task rather than replaying an existing one.
    pub created: bool,
}

/// Values required to create an idempotent pipeline task.
pub struct NewPipelineTask<'a> {
    pub tenant_id: &'a str,
    pub idempotency_key: &'a str,
    pub caller_reference: Option<&'a str>,
    pub audio_s3_uris: &'a [String],
}

/// Workflow states persisted in `pipeline_tasks.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(
    type_name = "pipeline_task_status",
    rename_all = "SCREAMING_SNAKE_CASE"
)]
pub enum PipelineTaskStatus {
    Pending,
    StartedAudioProcessing,
    AudioProcessingFinished,
    StartedAsr,
    AsrFinished,
    StartedModerationProcessing,
    ModerationProcessingFinished,
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
}

/// SQLx-backed access to pipeline tasks and their dispatch state.
#[derive(Clone)]
pub struct PipelineTaskStore {
    pool: PgPool,
}

impl PipelineTaskStore {
    /// Creates a store using the application's PostgreSQL connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Creates a task and its ordered inputs, or returns the task for the same
    /// tenant and idempotency key.
    pub async fn create_or_get(
        &self,
        task: NewPipelineTask<'_>,
    ) -> Result<PipelineTask, PipelineTaskError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(PipelineTaskError::CreateOrGet)?;

        // The no-op conflict update lets one statement return both newly
        // inserted and previously persisted tasks.
        let persisted = sqlx::query!(
            r#"
                INSERT INTO pipeline_tasks (tenant_id, idempotency_key, caller_reference)
                VALUES ($1, $2, $3)
                ON CONFLICT (tenant_id, idempotency_key) DO UPDATE
                    SET tenant_id = pipeline_tasks.tenant_id
                RETURNING task_id, tenant_id, idempotency_key, caller_reference,
                    status AS "status: PipelineTaskStatus", execution_arn, created_at, updated_at,
                    (xmax = 0) AS "created!"
            "#,
            task.tenant_id,
            task.idempotency_key,
            task.caller_reference,
        )
        .fetch_one(&mut *transaction)
        .await
        .map_err(PipelineTaskError::CreateOrGet)?;

        if persisted.created {
            // WITH ORDINALITY persists caller order without requiring sequence
            // numbers in the external API.
            sqlx::query!(
                r#"
                    INSERT INTO pipeline_task_inputs (task_id, sequence, audio_s3_uri)
                    SELECT $1, (ordinality - 1)::INTEGER, audio_s3_uri
                    FROM UNNEST($2::TEXT[]) WITH ORDINALITY AS input(audio_s3_uri, ordinality)
                "#,
                persisted.task_id,
                task.audio_s3_uris,
            )
            .execute(&mut *transaction)
            .await
            .map_err(PipelineTaskError::CreateOrGet)?;
        }

        let audio_s3_uris = sqlx::query_scalar!(
            r#"
                SELECT audio_s3_uri
                FROM pipeline_task_inputs
                WHERE task_id = $1
                ORDER BY sequence
            "#,
            persisted.task_id,
        )
        .fetch_all(&mut *transaction)
        .await
        .map_err(PipelineTaskError::CreateOrGet)?;

        transaction
            .commit()
            .await
            .map_err(PipelineTaskError::CreateOrGet)?;

        Ok(PipelineTask {
            task_id: persisted.task_id,
            tenant_id: persisted.tenant_id,
            idempotency_key: persisted.idempotency_key,
            caller_reference: persisted.caller_reference,
            status: persisted.status,
            execution_arn: persisted.execution_arn,
            created_at: persisted.created_at,
            updated_at: persisted.updated_at,
            audio_s3_uris,
            created: persisted.created,
        })
    }

    /// Records a successful Step Functions start and releases the dispatch lease.
    pub async fn record_execution(
        &self,
        task_id: i32,
        execution_arn: &str,
    ) -> Result<(), PipelineTaskError> {
        sqlx::query!(
            r#"
                UPDATE pipeline_tasks
                SET execution_arn = $2,
                    dispatch_started_at = NULL,
                    error_code = NULL,
                    error_message = NULL,
                    updated_at = NOW()
                WHERE task_id = $1 AND execution_arn IS NULL
            "#,
            task_id,
            execution_arn,
        )
        .execute(&self.pool)
        .await
        .map_err(PipelineTaskError::RecordExecution)?;

        Ok(())
    }

    /// Atomically leases an undispatched task and returns its attempt number.
    ///
    /// An expired lease permits recovery after a Lambda exits while dispatching.
    /// Returning `None` means another invocation owns the lease, the task was
    /// already dispatched, or the retry limit has been reached.
    pub async fn claim_dispatch(&self, task_id: i32) -> Result<Option<i32>, PipelineTaskError> {
        let attempt_count = sqlx::query_scalar!(
            r#"
                UPDATE pipeline_tasks
                SET dispatch_started_at = NOW(),
                    attempt_count = attempt_count + 1,
                    updated_at = NOW()
                WHERE task_id = $1
                    AND execution_arn IS NULL
                    AND attempt_count < 3
                    AND (
                        dispatch_started_at IS NULL
                        OR dispatch_started_at < NOW() - INTERVAL '5 minutes'
                    )
                RETURNING attempt_count
            "#,
            task_id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(PipelineTaskError::ClaimDispatch)?;

        Ok(attempt_count)
    }

    /// Releases the dispatch lease and records the failure for a later retry.
    /// The third failure moves the task to its terminal `FAILED` state.
    pub async fn record_dispatch_failure(
        &self,
        task_id: i32,
        error_message: &str,
    ) -> Result<(), PipelineTaskError> {
        sqlx::query!(
            r#"
                UPDATE pipeline_tasks
                SET dispatch_started_at = NULL,
                    error_code = 'DISPATCH_FAILED',
                    error_message = $2,
                    status = CASE
                        WHEN attempt_count >= 3 THEN 'FAILED'::pipeline_task_status
                        ELSE status
                    END,
                    completed_at = CASE
                        WHEN attempt_count >= 3 THEN NOW()
                        ELSE completed_at
                    END,
                    updated_at = NOW()
                WHERE task_id = $1 AND execution_arn IS NULL
            "#,
            task_id,
            error_message,
        )
        .execute(&self.pool)
        .await
        .map_err(PipelineTaskError::RecordDispatchFailure)?;

        Ok(())
    }
}
