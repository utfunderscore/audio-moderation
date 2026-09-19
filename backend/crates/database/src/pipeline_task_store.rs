use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

/// A Lambda invocation has ten seconds to dispatch an execution. Keep the
/// lease short enough that Lambda's asynchronous retry can reclaim a task when
/// an invocation exits before persisting its Step Functions response.
pub const DEFAULT_DISPATCH_LEASE: Duration = Duration::from_secs(30);

/// Errors returned while accessing or dispatching persisted pipeline tasks.
#[derive(Debug, thiserror::Error)]
pub enum PipelineTaskError {
    #[error("failed to create or retrieve pipeline task")]
    CreateOrGet(#[source] sqlx::Error),

    #[error("failed to record pipeline task execution")]
    RecordExecution(#[source] sqlx::Error),

    #[error("pipeline task {task_id} already has a different execution ARN")]
    ExecutionConflict { task_id: i32 },

    #[error("pipeline task {0} cannot record an execution")]
    ExecutionNotRecordable(i32),

    #[error("failed to record ASR task ID")]
    RecordAsrTask(#[source] sqlx::Error),

    #[error("pipeline task already has a different ASR task ID")]
    AsrTaskConflict,

    #[error("failed to record moderation task ID")]
    RecordModerationTask(#[source] sqlx::Error),

    #[error("pipeline task already has a different moderation task ID")]
    ModerationTaskConflict,

    #[error("failed to claim pipeline task dispatch")]
    ClaimDispatch(#[source] sqlx::Error),

    #[error("failed to record pipeline task dispatch failure")]
    RecordDispatchFailure(#[source] sqlx::Error),

    #[error("failed to update pipeline task lifecycle")]
    Lifecycle(#[source] sqlx::Error),

    #[error("pipeline task {task_id} cannot perform this transition from {status:?}")]
    TransitionConflict {
        task_id: i32,
        status: PipelineTaskStatus,
    },

    #[error("pipeline task {0} does not exist")]
    TaskNotFound(i32),

    #[error("pipeline task already has a different stitched audio URI")]
    AudioOutputConflict,

    #[error("pipeline task already has a different transcription")]
    TranscriptionConflict,

    #[error("callback token does not match a pipeline callback attempt")]
    CallbackTokenNotFound,

    #[error("callback token belongs to a different pipeline task")]
    CallbackTaskConflict,

    #[error("callback token belongs to a different pipeline step")]
    CallbackStepConflict,

    #[error("pipeline task already has a different moderation result")]
    ModerationConflict,

    #[error("moderation callback job ID does not match its task token")]
    ModerationJobConflict,

    #[error("failed to retrieve pipeline task")]
    Get(#[source] sqlx::Error),

    #[error("pipeline task was not found")]
    NotFound,
}

/// Scores produced by the moderation pipeline step.
#[derive(Debug, Clone, Copy, PartialEq, sqlx::FromRow)]
pub struct ModerationResult {
    pub sexual: f64,
    pub hate_or_discrimination: f64,
    pub harassment_or_abuse: f64,
    pub violence_or_threats: f64,
    pub asking_for_pii: f64,
}

/// The callback-capable workflow steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(
    type_name = "pipeline_callback_step",
    rename_all = "SCREAMING_SNAKE_CASE"
)]
pub enum PipelineCallbackStep {
    Transcription,
    Moderation,
}

/// The task and step identified by a callback token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::FromRow)]
pub struct CallbackAttempt {
    pub task_id: i32,
    pub step: PipelineCallbackStep,
}

/// A pipeline task together with its ordered audio inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineTask {
    pub task_id: i32,
    pub evaluation_id: String,
    pub tenant_id: String,
    pub idempotency_key: String,
    pub caller_reference: Option<String>,
    pub status: PipelineTaskStatus,
    pub execution_arn: Option<String>,
    pub asr_task_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub audio_s3_uris: Vec<String>,
    /// Whether this call created the task rather than replaying an existing one.
    pub created: bool,
}

/// The outcome of persisting a Step Functions execution ARN.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordExecutionResult {
    /// This call recorded the execution ARN and released the dispatch lease.
    Recorded,
    /// A concurrent or retried call had already recorded this exact ARN.
    AlreadyRecorded,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PipelineTaskDetails {
    pub task_id: i32,
    pub evaluation_id: Uuid,
    pub caller_reference: Option<String>,
    pub status: PipelineTaskStatus,
    pub outcome: Option<PipelineTaskOutcome>,
    pub audio_s3_uris: Vec<String>,
    pub audio_processing: AudioProcessingTaskDetails,
    pub transcription: TranscriptionTaskDetails,
    pub moderation: ModerationTaskDetails,
    pub events: Vec<PipelineTaskEventDetails>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineStepErrorDetails {
    pub code: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioProcessingTaskDetails {
    pub status: PipelineStepStatus,
    pub stitched_audio_s3_uri: Option<String>,
    pub error: PipelineStepErrorDetails,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptionTaskDetails {
    pub status: PipelineStepStatus,
    pub external_task_id: Option<String>,
    pub transcript: Option<String>,
    pub error: PipelineStepErrorDetails,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModerationTaskDetails {
    pub status: PipelineStepStatus,
    pub external_task_id: Option<String>,
    pub scores: Option<ModerationResult>,
    pub error: PipelineStepErrorDetails,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct PipelineTaskEventDetails {
    pub event_id: i64,
    pub event_name: String,
    pub created_at: DateTime<Utc>,
}

/// Values required to create an idempotent pipeline task.
pub struct NewPipelineTask<'a> {
    pub tenant_id: &'a str,
    pub idempotency_key: &'a str,
    pub caller_reference: Option<&'a str>,
    pub audio_s3_uris: &'a [String],
}

#[derive(sqlx::FromRow)]
struct PersistedPipelineTask {
    task_id: i32,
    evaluation_id: String,
    tenant_id: String,
    idempotency_key: String,
    caller_reference: Option<String>,
    execution_arn: Option<String>,
    asr_task_id: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    created: bool,
}

#[derive(sqlx::FromRow)]
struct PersistedExecution {
    execution_arn: Option<String>,
}

/// Workflow state derived from the pipeline outcome and individual step rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(
    type_name = "pipeline_task_outcome",
    rename_all = "SCREAMING_SNAKE_CASE"
)]
pub enum PipelineTaskOutcome {
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(
    type_name = "pipeline_step_status",
    rename_all = "SCREAMING_SNAKE_CASE"
)]
pub enum PipelineStepStatus {
    Pending,
    Processing,
    Completed,
    Failed,
}

#[derive(sqlx::FromRow)]
struct PipelineStepState {
    audio_status: PipelineStepStatus,
    transcription_status: PipelineStepStatus,
    moderation_status: PipelineStepStatus,
}

#[derive(sqlx::FromRow)]
struct PipelineTaskDetailsRow {
    task_id: i32,
    evaluation_id: Uuid,
    caller_reference: Option<String>,
    outcome: Option<PipelineTaskOutcome>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    completed_at: Option<DateTime<Utc>>,
    audio_status: PipelineStepStatus,
    stitched_audio_s3_uri: Option<String>,
    audio_error_code: Option<String>,
    audio_error_message: Option<String>,
    audio_started_at: Option<DateTime<Utc>>,
    audio_completed_at: Option<DateTime<Utc>>,
    audio_updated_at: DateTime<Utc>,
    transcription_status: PipelineStepStatus,
    transcription_external_task_id: Option<String>,
    transcript: Option<String>,
    transcription_error_code: Option<String>,
    transcription_error_message: Option<String>,
    transcription_started_at: Option<DateTime<Utc>>,
    transcription_completed_at: Option<DateTime<Utc>>,
    transcription_updated_at: DateTime<Utc>,
    moderation_status: PipelineStepStatus,
    moderation_external_task_id: Option<String>,
    sexual: Option<f64>,
    hate_or_discrimination: Option<f64>,
    harassment_or_abuse: Option<f64>,
    violence_or_threats: Option<f64>,
    asking_for_pii: Option<f64>,
    moderation_error_code: Option<String>,
    moderation_error_message: Option<String>,
    moderation_started_at: Option<DateTime<Utc>>,
    moderation_completed_at: Option<DateTime<Utc>>,
    moderation_updated_at: DateTime<Utc>,
}

impl PipelineStepState {
    fn status(&self, outcome: Option<PipelineTaskOutcome>) -> PipelineTaskStatus {
        match outcome {
            Some(PipelineTaskOutcome::Succeeded) => PipelineTaskStatus::Succeeded,
            Some(PipelineTaskOutcome::Failed) => PipelineTaskStatus::Failed,
            Some(PipelineTaskOutcome::TimedOut) => PipelineTaskStatus::TimedOut,
            Some(PipelineTaskOutcome::Cancelled) => PipelineTaskStatus::Cancelled,
            None if self.audio_status == PipelineStepStatus::Failed
                || self.transcription_status == PipelineStepStatus::Failed
                || self.moderation_status == PipelineStepStatus::Failed =>
            {
                PipelineTaskStatus::Failed
            }
            None if self.moderation_status == PipelineStepStatus::Completed => {
                PipelineTaskStatus::ModerationProcessingFinished
            }
            None if self.moderation_status == PipelineStepStatus::Processing => {
                PipelineTaskStatus::StartedModerationProcessing
            }
            None if self.transcription_status == PipelineStepStatus::Completed => {
                PipelineTaskStatus::AsrFinished
            }
            None if self.transcription_status == PipelineStepStatus::Processing => {
                PipelineTaskStatus::StartedAsr
            }
            None if self.audio_status == PipelineStepStatus::Completed => {
                PipelineTaskStatus::AudioProcessingFinished
            }
            None if self.audio_status == PipelineStepStatus::Processing => {
                PipelineTaskStatus::StartedAudioProcessing
            }
            None => PipelineTaskStatus::Pending,
        }
    }
}

/// SQLx-backed access to pipeline tasks and their dispatch state.
#[derive(Clone)]
pub struct PipelineTaskStore {
    pool: PgPool,
    dispatch_lease: Duration,
}

impl PipelineTaskStore {
    /// Creates a store using the application's PostgreSQL connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self::with_dispatch_lease(pool, DEFAULT_DISPATCH_LEASE)
    }

    /// Creates a store with an explicit dispatch lease duration.
    ///
    /// This is primarily useful to align a deployment with its Lambda retry
    /// policy and to exercise expired-lease recovery in focused tests.
    pub fn with_dispatch_lease(pool: PgPool, dispatch_lease: Duration) -> Self {
        assert!(
            !dispatch_lease.is_zero(),
            "dispatch lease duration must be greater than zero"
        );
        Self {
            pool,
            dispatch_lease,
        }
    }

    pub async fn get_details(
        &self,
        evaluation_id: Uuid,
        tenant_id: &str,
    ) -> Result<PipelineTaskDetails, PipelineTaskError> {
        let mut transaction = self.pool.begin().await.map_err(PipelineTaskError::Get)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *transaction)
            .await
            .map_err(PipelineTaskError::Get)?;

        let row = sqlx::query_as::<_, PipelineTaskDetailsRow>(
            r#"
                SELECT pipeline.task_id, pipeline.evaluation_id, pipeline.caller_reference,
                    pipeline.outcome, pipeline.created_at, pipeline.updated_at,
                    pipeline.completed_at,
                    audio.status AS audio_status, audio.stitched_audio_s3_uri,
                    audio.error_code AS audio_error_code,
                    audio.error_message AS audio_error_message,
                    audio.started_at AS audio_started_at,
                    audio.completed_at AS audio_completed_at,
                    audio.updated_at AS audio_updated_at,
                    transcription.status AS transcription_status,
                    transcription.external_task_id AS transcription_external_task_id,
                    transcription.transcript,
                    transcription.error_code AS transcription_error_code,
                    transcription.error_message AS transcription_error_message,
                    transcription.started_at AS transcription_started_at,
                    transcription.completed_at AS transcription_completed_at,
                    transcription.updated_at AS transcription_updated_at,
                    moderation.status AS moderation_status,
                    moderation.external_task_id AS moderation_external_task_id,
                    moderation.sexual, moderation.hate_or_discrimination,
                    moderation.harassment_or_abuse, moderation.violence_or_threats,
                    moderation.asking_for_pii,
                    moderation.error_code AS moderation_error_code,
                    moderation.error_message AS moderation_error_message,
                    moderation.started_at AS moderation_started_at,
                    moderation.completed_at AS moderation_completed_at,
                    moderation.updated_at AS moderation_updated_at
                FROM pipeline_tasks pipeline
                JOIN audio_processing_tasks audio USING (task_id)
                JOIN transcription_tasks transcription USING (task_id)
                JOIN moderation_tasks moderation USING (task_id)
                WHERE pipeline.evaluation_id = $1 AND pipeline.tenant_id = $2
            "#,
        )
        .bind(evaluation_id)
        .bind(tenant_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(PipelineTaskError::Get)?
        .ok_or(PipelineTaskError::NotFound)?;

        let audio_s3_uris = sqlx::query_scalar::<_, String>(
            "SELECT audio_s3_uri FROM pipeline_task_inputs WHERE task_id = $1 ORDER BY sequence",
        )
        .bind(row.task_id)
        .fetch_all(&mut *transaction)
        .await
        .map_err(PipelineTaskError::Get)?;
        let events = sqlx::query_as::<_, PipelineTaskEventDetails>(
            r#"
                SELECT event_id, event_name, created_at
                FROM pipeline_task_events
                WHERE task_id = $1
                ORDER BY event_id
            "#,
        )
        .bind(row.task_id)
        .fetch_all(&mut *transaction)
        .await
        .map_err(PipelineTaskError::Get)?;
        transaction.commit().await.map_err(PipelineTaskError::Get)?;

        let status = PipelineStepState {
            audio_status: row.audio_status,
            transcription_status: row.transcription_status,
            moderation_status: row.moderation_status,
        }
        .status(row.outcome);
        let scores = match (
            row.sexual,
            row.hate_or_discrimination,
            row.harassment_or_abuse,
            row.violence_or_threats,
            row.asking_for_pii,
        ) {
            (Some(sexual), Some(hate), Some(harassment), Some(violence), Some(pii)) => {
                Some(ModerationResult {
                    sexual,
                    hate_or_discrimination: hate,
                    harassment_or_abuse: harassment,
                    violence_or_threats: violence,
                    asking_for_pii: pii,
                })
            }
            _ => None,
        };

        Ok(PipelineTaskDetails {
            task_id: row.task_id,
            evaluation_id: row.evaluation_id,
            caller_reference: row.caller_reference,
            status,
            outcome: row.outcome,
            audio_s3_uris,
            audio_processing: AudioProcessingTaskDetails {
                status: row.audio_status,
                stitched_audio_s3_uri: row.stitched_audio_s3_uri,
                error: PipelineStepErrorDetails {
                    code: row.audio_error_code,
                    message: row.audio_error_message,
                },
                started_at: row.audio_started_at,
                completed_at: row.audio_completed_at,
                updated_at: row.audio_updated_at,
            },
            transcription: TranscriptionTaskDetails {
                status: row.transcription_status,
                external_task_id: row.transcription_external_task_id,
                transcript: row.transcript,
                error: PipelineStepErrorDetails {
                    code: row.transcription_error_code,
                    message: row.transcription_error_message,
                },
                started_at: row.transcription_started_at,
                completed_at: row.transcription_completed_at,
                updated_at: row.transcription_updated_at,
            },
            moderation: ModerationTaskDetails {
                status: row.moderation_status,
                external_task_id: row.moderation_external_task_id,
                scores,
                error: PipelineStepErrorDetails {
                    code: row.moderation_error_code,
                    message: row.moderation_error_message,
                },
                started_at: row.moderation_started_at,
                completed_at: row.moderation_completed_at,
                updated_at: row.moderation_updated_at,
            },
            events,
            created_at: row.created_at,
            updated_at: row.updated_at,
            completed_at: row.completed_at,
        })
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
        let persisted = sqlx::query_as::<_, PersistedPipelineTask>(
            r#"
                INSERT INTO pipeline_tasks (tenant_id, idempotency_key, caller_reference)
                VALUES ($1, $2, $3)
                ON CONFLICT (tenant_id, idempotency_key) DO UPDATE
                    SET tenant_id = pipeline_tasks.tenant_id
                RETURNING task_id, evaluation_id::TEXT AS evaluation_id, tenant_id,
                    idempotency_key, caller_reference,
                    execution_arn,
                    (
                        SELECT external_task_id
                        FROM transcription_tasks
                        WHERE transcription_tasks.task_id = pipeline_tasks.task_id
                    ) AS asr_task_id,
                    created_at, updated_at,
                    (xmax = 0) AS created
            "#,
        )
        .bind(task.tenant_id)
        .bind(task.idempotency_key)
        .bind(task.caller_reference)
        .fetch_one(&mut *transaction)
        .await
        .map_err(PipelineTaskError::CreateOrGet)?;

        if persisted.created {
            sqlx::query(
                r#"
                    WITH audio_processing AS (
                        INSERT INTO audio_processing_tasks (task_id)
                        VALUES ($1)
                        RETURNING task_id
                    ), transcription AS (
                        INSERT INTO transcription_tasks (task_id)
                        VALUES ($1)
                        RETURNING task_id
                    )
                    INSERT INTO moderation_tasks (task_id)
                    VALUES ($1)
                "#,
            )
            .bind(persisted.task_id)
            .execute(&mut *transaction)
            .await
            .map_err(PipelineTaskError::CreateOrGet)?;

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

        let status = task_status_for_update(&mut transaction, persisted.task_id).await?;

        transaction
            .commit()
            .await
            .map_err(PipelineTaskError::CreateOrGet)?;

        Ok(PipelineTask {
            task_id: persisted.task_id,
            evaluation_id: persisted.evaluation_id,
            tenant_id: persisted.tenant_id,
            idempotency_key: persisted.idempotency_key,
            caller_reference: persisted.caller_reference,
            status,
            execution_arn: persisted.execution_arn,
            asr_task_id: persisted.asr_task_id,
            created_at: persisted.created_at,
            updated_at: persisted.updated_at,
            audio_s3_uris,
            created: persisted.created,
        })
    }

    /// Records a successful Step Functions start and releases the dispatch lease.
    ///
    /// A retry that observes the same ARN is reported as `AlreadyRecorded`; a
    /// different ARN or a task that cannot accept an execution is an error.
    pub async fn record_execution(
        &self,
        task_id: i32,
        execution_arn: &str,
    ) -> Result<RecordExecutionResult, PipelineTaskError> {
        let recorded = sqlx::query_scalar::<_, i32>(
            r#"
                UPDATE pipeline_tasks
                SET execution_arn = $2,
                    dispatch_started_at = NULL,
                    updated_at = NOW()
                WHERE task_id = $1 AND execution_arn IS NULL AND outcome IS NULL
                RETURNING task_id
            "#,
        )
        .bind(task_id)
        .bind(execution_arn)
        .fetch_optional(&self.pool)
        .await
        .map_err(PipelineTaskError::RecordExecution)?;

        if recorded.is_some() {
            return Ok(RecordExecutionResult::Recorded);
        }

        let persisted = sqlx::query_as::<_, PersistedExecution>(
            "SELECT execution_arn FROM pipeline_tasks WHERE task_id = $1",
        )
        .bind(task_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(PipelineTaskError::RecordExecution)?
        .ok_or(PipelineTaskError::TaskNotFound(task_id))?;

        match persisted.execution_arn {
            Some(existing) if existing == execution_arn => {
                Ok(RecordExecutionResult::AlreadyRecorded)
            }
            Some(_) => Err(PipelineTaskError::ExecutionConflict { task_id }),
            None => Err(PipelineTaskError::ExecutionNotRecordable(task_id)),
        }
    }

    /// Records the external ASR task ID returned for a pipeline task.
    ///
    /// Repeating the same ID is safe after a retry, but a different ID would
    /// indicate an idempotency failure at the external service.
    pub async fn record_asr_task(
        &self,
        task_id: i32,
        asr_task_id: &str,
    ) -> Result<(), PipelineTaskError> {
        let result = sqlx::query(
            r#"
                UPDATE transcription_tasks
                SET external_task_id = $2,
                    updated_at = NOW()
                WHERE task_id = $1
                    AND (external_task_id IS NULL OR external_task_id = $2)
            "#,
        )
        .bind(task_id)
        .bind(asr_task_id)
        .execute(&self.pool)
        .await
        .map_err(PipelineTaskError::RecordAsrTask)?;

        if result.rows_affected() == 0 {
            return Err(PipelineTaskError::AsrTaskConflict);
        }

        Ok(())
    }

    /// Records the external moderation task ID returned for a pipeline task.
    /// Repeating the same ID is safe after a retry.
    pub async fn record_moderation_task(
        &self,
        task_id: i32,
        moderation_task_id: &str,
    ) -> Result<(), PipelineTaskError> {
        let result = sqlx::query(
            r#"
                UPDATE moderation_tasks
                SET external_task_id = $2,
                    updated_at = NOW()
                WHERE task_id = $1
                    AND (external_task_id IS NULL OR external_task_id = $2)
            "#,
        )
        .bind(task_id)
        .bind(moderation_task_id)
        .execute(&self.pool)
        .await
        .map_err(PipelineTaskError::RecordModerationTask)?;

        if result.rows_affected() == 0 {
            return Err(PipelineTaskError::ModerationTaskConflict);
        }

        Ok(())
    }

    /// Starts ASR and records the digest of the workflow task token before the
    /// external service can receive it. This makes an immediate callback safe.
    pub async fn start_asr(&self, task_id: i32, task_token: &str) -> Result<(), PipelineTaskError> {
        let token_hash = task_token_hash(task_token);
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(PipelineTaskError::Lifecycle)?;
        let status = task_status_for_update(&mut transaction, task_id).await?;
        match status {
            PipelineTaskStatus::AudioProcessingFinished => {
                let step_result = sqlx::query(
                    "UPDATE transcription_tasks SET status = 'PROCESSING', \
                     started_at = COALESCE(started_at, NOW()), updated_at = NOW() \
                     WHERE task_id = $1 AND status = 'PENDING'",
                )
                .bind(task_id)
                .execute(&mut *transaction)
                .await
                .map_err(PipelineTaskError::Lifecycle)?;
                if step_result.rows_affected() != 1 {
                    return Err(PipelineTaskError::TransitionConflict { task_id, status });
                }
            }
            PipelineTaskStatus::StartedAsr => {}
            status => return Err(PipelineTaskError::TransitionConflict { task_id, status }),
        }

        let insert = sqlx::query(
            "INSERT INTO pipeline_callback_attempts (task_id, step, task_token_hash) \
             VALUES ($1, $2, $3) ON CONFLICT (task_id, step, task_token_hash) DO NOTHING",
        )
        .bind(task_id)
        .bind(PipelineCallbackStep::Transcription)
        .bind(&token_hash)
        .execute(&mut *transaction)
        .await
        .map_err(PipelineTaskError::Lifecycle)?;
        if insert.rows_affected() == 0 {
            let owner = sqlx::query_as::<_, CallbackAttempt>(
                "SELECT task_id, step FROM pipeline_callback_attempts WHERE task_token_hash = $1",
            )
            .bind(&token_hash)
            .fetch_one(&mut *transaction)
            .await
            .map_err(PipelineTaskError::Lifecycle)?;
            if owner.task_id != task_id {
                return Err(PipelineTaskError::CallbackTaskConflict);
            }
            if owner.step != PipelineCallbackStep::Transcription {
                return Err(PipelineTaskError::CallbackStepConflict);
            }
        }
        sqlx::query(
            "UPDATE pipeline_tasks SET updated_at = NOW() WHERE task_id = $1 AND outcome IS NULL",
        )
        .bind(task_id)
        .execute(&mut *transaction)
        .await
        .map_err(PipelineTaskError::Lifecycle)?;
        transaction
            .commit()
            .await
            .map_err(PipelineTaskError::Lifecycle)
    }

    /// Starts moderation and records the callback token digest before an
    /// external moderation service can receive the token.
    pub async fn start_moderation(
        &self,
        task_id: i32,
        task_token: &str,
    ) -> Result<(), PipelineTaskError> {
        let token_hash = task_token_hash(task_token);
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(PipelineTaskError::Lifecycle)?;
        let status = task_status_for_update(&mut transaction, task_id).await?;
        match status {
            PipelineTaskStatus::AsrFinished => {
                let step_result = sqlx::query(
                    "UPDATE moderation_tasks SET status = 'PROCESSING', \
                     started_at = COALESCE(started_at, NOW()), updated_at = NOW() \
                     WHERE task_id = $1 AND status = 'PENDING'",
                )
                .bind(task_id)
                .execute(&mut *transaction)
                .await
                .map_err(PipelineTaskError::Lifecycle)?;
                if step_result.rows_affected() != 1 {
                    return Err(PipelineTaskError::TransitionConflict { task_id, status });
                }
            }
            PipelineTaskStatus::StartedModerationProcessing => {}
            status => return Err(PipelineTaskError::TransitionConflict { task_id, status }),
        }

        insert_callback_attempt(
            &mut transaction,
            task_id,
            PipelineCallbackStep::Moderation,
            &token_hash,
        )
        .await?;
        sqlx::query(
            "UPDATE pipeline_tasks SET updated_at = NOW() WHERE task_id = $1 AND outcome IS NULL",
        )
        .bind(task_id)
        .execute(&mut *transaction)
        .await
        .map_err(PipelineTaskError::Lifecycle)?;
        transaction
            .commit()
            .await
            .map_err(PipelineTaskError::Lifecycle)
    }

    /// Marks a pending task as being processed by the audio worker.
    pub async fn start_audio_processing(&self, task_id: i32) -> Result<(), PipelineTaskError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(PipelineTaskError::Lifecycle)?;
        let status = task_status_for_update(&mut transaction, task_id).await?;

        match status {
            PipelineTaskStatus::Pending => {
                let step_result = sqlx::query(
                    "UPDATE audio_processing_tasks SET status = 'PROCESSING', \
                     started_at = COALESCE(started_at, NOW()), updated_at = NOW() \
                     WHERE task_id = $1 AND status = 'PENDING'",
                )
                .bind(task_id)
                .execute(&mut *transaction)
                .await
                .map_err(PipelineTaskError::Lifecycle)?;
                if step_result.rows_affected() != 1 {
                    return Err(PipelineTaskError::TransitionConflict { task_id, status });
                }
                let result = sqlx::query(
                    "UPDATE pipeline_tasks SET updated_at = NOW() \
                     WHERE task_id = $1 AND outcome IS NULL",
                )
                .bind(task_id)
                .execute(&mut *transaction)
                .await
                .map_err(PipelineTaskError::Lifecycle)?;
                if result.rows_affected() != 1 {
                    return Err(PipelineTaskError::TransitionConflict { task_id, status });
                }
            }
            PipelineTaskStatus::StartedAudioProcessing
            | PipelineTaskStatus::AudioProcessingFinished
            | PipelineTaskStatus::StartedAsr
            | PipelineTaskStatus::AsrFinished
            | PipelineTaskStatus::StartedModerationProcessing
            | PipelineTaskStatus::ModerationProcessingFinished => {}
            status => return Err(PipelineTaskError::TransitionConflict { task_id, status }),
        }

        transaction
            .commit()
            .await
            .map_err(PipelineTaskError::Lifecycle)
    }

    /// Persists the stitched artifact and advances the task in one transaction.
    pub async fn complete_audio_processing(
        &self,
        task_id: i32,
        stitched_audio_s3_uri: &str,
    ) -> Result<(), PipelineTaskError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(PipelineTaskError::Lifecycle)?;
        let status = task_status_for_update(&mut transaction, task_id).await?;
        let existing = sqlx::query_scalar::<_, Option<String>>(
            "SELECT stitched_audio_s3_uri FROM audio_processing_tasks WHERE task_id = $1",
        )
        .bind(task_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(PipelineTaskError::Lifecycle)?;

        if let Some(existing) = existing {
            if existing != stitched_audio_s3_uri {
                return Err(PipelineTaskError::AudioOutputConflict);
            }
            transaction
                .commit()
                .await
                .map_err(PipelineTaskError::Lifecycle)?;
            return Ok(());
        }
        if status != PipelineTaskStatus::StartedAudioProcessing {
            return Err(PipelineTaskError::TransitionConflict { task_id, status });
        }

        let step_result = sqlx::query(
            "UPDATE audio_processing_tasks SET status = 'COMPLETED', \
             stitched_audio_s3_uri = $2, completed_at = NOW(), updated_at = NOW() \
             WHERE task_id = $1 AND status = 'PROCESSING' AND stitched_audio_s3_uri IS NULL",
        )
        .bind(task_id)
        .bind(stitched_audio_s3_uri)
        .execute(&mut *transaction)
        .await
        .map_err(PipelineTaskError::Lifecycle)?;
        if step_result.rows_affected() != 1 {
            return Err(PipelineTaskError::TransitionConflict { task_id, status });
        }
        let result = sqlx::query(
            "UPDATE pipeline_tasks SET updated_at = NOW() \
             WHERE task_id = $1 AND outcome IS NULL",
        )
        .bind(task_id)
        .execute(&mut *transaction)
        .await
        .map_err(PipelineTaskError::Lifecycle)?;
        if result.rows_affected() != 1 {
            return Err(PipelineTaskError::TransitionConflict { task_id, status });
        }
        transaction
            .commit()
            .await
            .map_err(PipelineTaskError::Lifecycle)
    }

    /// Records a non-retryable audio worker failure as a terminal pipeline
    /// failure. Infrastructure retries are handled by Step Functions and do
    /// not call this method.
    pub async fn fail_audio_processing(
        &self,
        task_id: i32,
        error_code: &str,
        error_message: &str,
    ) -> Result<(), PipelineTaskError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(PipelineTaskError::Lifecycle)?;
        let status = task_status_for_update(&mut transaction, task_id).await?;
        match status {
            PipelineTaskStatus::StartedAudioProcessing => {
                sqlx::query(
                    "UPDATE audio_processing_tasks SET status = 'FAILED', error_code = $2, \
                     error_message = $3, completed_at = NOW(), updated_at = NOW() \
                     WHERE task_id = $1 AND status = 'PROCESSING'",
                )
                .bind(task_id)
                .bind(error_code)
                .bind(error_message)
                .execute(&mut *transaction)
                .await
                .map_err(PipelineTaskError::Lifecycle)?;
                sqlx::query(
                    "UPDATE pipeline_tasks SET outcome = 'FAILED', completed_at = NOW(), updated_at = NOW() \
                     WHERE task_id = $1 AND outcome IS NULL",
                )
                .bind(task_id)
                .execute(&mut *transaction)
                .await
                .map_err(PipelineTaskError::Lifecycle)?;
            }
            PipelineTaskStatus::Failed => {}
            status => return Err(PipelineTaskError::TransitionConflict { task_id, status }),
        }
        transaction
            .commit()
            .await
            .map_err(PipelineTaskError::Lifecycle)
    }

    /// Persists an ASR result and advances the task in one transaction.
    pub async fn complete_asr(
        &self,
        task_id: i32,
        task_token: &str,
        asr_task_id: &str,
        transcription: &str,
    ) -> Result<(), PipelineTaskError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(PipelineTaskError::Lifecycle)?;
        let status = task_status_for_update(&mut transaction, task_id).await?;
        validate_callback_token(
            &mut transaction,
            task_id,
            PipelineCallbackStep::Transcription,
            task_token,
        )
        .await?;
        let persisted_asr_task_id = sqlx::query_scalar::<_, Option<String>>(
            "SELECT external_task_id FROM transcription_tasks WHERE task_id = $1",
        )
        .bind(task_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(PipelineTaskError::Lifecycle)?;
        if persisted_asr_task_id
            .as_deref()
            .is_some_and(|id| id != asr_task_id)
        {
            return Err(PipelineTaskError::AsrTaskConflict);
        }
        let existing = sqlx::query_scalar::<_, Option<String>>(
            "SELECT transcript FROM transcription_tasks WHERE task_id = $1",
        )
        .bind(task_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(PipelineTaskError::Lifecycle)?;
        if let Some(existing) = existing {
            if existing != transcription {
                return Err(PipelineTaskError::TranscriptionConflict);
            }
            transaction
                .commit()
                .await
                .map_err(PipelineTaskError::Lifecycle)?;
            return Ok(());
        }
        if status != PipelineTaskStatus::StartedAsr {
            return Err(PipelineTaskError::TransitionConflict { task_id, status });
        }

        let transcription_update = sqlx::query(
            "UPDATE transcription_tasks SET status = 'COMPLETED', external_task_id = $2, \
             transcript = $3, completed_at = NOW(), updated_at = NOW() \
             WHERE task_id = $1 AND status = 'PROCESSING' \
             AND transcript IS NULL AND (external_task_id IS NULL OR external_task_id = $2)",
        )
        .bind(task_id)
        .bind(asr_task_id)
        .bind(transcription)
        .execute(&mut *transaction)
        .await
        .map_err(PipelineTaskError::Lifecycle)?;
        if transcription_update.rows_affected() != 1 {
            return Err(PipelineTaskError::AsrTaskConflict);
        }
        let result = sqlx::query(
            "UPDATE pipeline_tasks SET updated_at = NOW() \
             WHERE task_id = $1 AND outcome IS NULL",
        )
        .bind(task_id)
        .execute(&mut *transaction)
        .await
        .map_err(PipelineTaskError::Lifecycle)?;
        if result.rows_affected() != 1 {
            return Err(PipelineTaskError::TransitionConflict { task_id, status });
        }
        transaction
            .commit()
            .await
            .map_err(PipelineTaskError::Lifecycle)
    }

    /// Persists a moderation result and advances the moderation step in one
    /// transaction. Replaying the exact same callback is safe.
    pub async fn complete_moderation(
        &self,
        task_id: i32,
        task_token: &str,
        moderation_task_id: &str,
        result: ModerationResult,
    ) -> Result<(), PipelineTaskError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(PipelineTaskError::Lifecycle)?;
        let status = task_status_for_update(&mut transaction, task_id).await?;
        validate_callback_token(
            &mut transaction,
            task_id,
            PipelineCallbackStep::Moderation,
            task_token,
        )
        .await?;
        let persisted_moderation_task_id = sqlx::query_scalar::<_, Option<String>>(
            "SELECT external_task_id FROM moderation_tasks WHERE task_id = $1",
        )
        .bind(task_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(PipelineTaskError::Lifecycle)?;
        if persisted_moderation_task_id
            .as_deref()
            .is_some_and(|id| id != moderation_task_id)
        {
            return Err(PipelineTaskError::ModerationConflict);
        }
        let existing = sqlx::query_as::<_, ModerationResult>(
            "SELECT sexual, hate_or_discrimination, harassment_or_abuse, violence_or_threats, \
             asking_for_pii FROM moderation_tasks WHERE task_id = $1 \
             AND status = 'COMPLETED'",
        )
        .bind(task_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(PipelineTaskError::Lifecycle)?;
        if let Some(existing) = existing {
            if existing != result {
                return Err(PipelineTaskError::ModerationConflict);
            }
            transaction
                .commit()
                .await
                .map_err(PipelineTaskError::Lifecycle)?;
            return Ok(());
        }
        if status != PipelineTaskStatus::StartedModerationProcessing {
            return Err(PipelineTaskError::TransitionConflict { task_id, status });
        }

        let step_result = sqlx::query(
            "UPDATE moderation_tasks SET status = 'COMPLETED', external_task_id = $2, sexual = $3, \
             hate_or_discrimination = $4, harassment_or_abuse = $5, \
             violence_or_threats = $6, asking_for_pii = $7, completed_at = NOW(), \
             updated_at = NOW() WHERE task_id = $1 AND status = 'PROCESSING' \
             AND (external_task_id IS NULL OR external_task_id = $2)",
        )
        .bind(task_id)
        .bind(moderation_task_id)
        .bind(result.sexual)
        .bind(result.hate_or_discrimination)
        .bind(result.harassment_or_abuse)
        .bind(result.violence_or_threats)
        .bind(result.asking_for_pii)
        .execute(&mut *transaction)
        .await
        .map_err(PipelineTaskError::Lifecycle)?;
        if step_result.rows_affected() != 1 {
            return Err(PipelineTaskError::TransitionConflict { task_id, status });
        }
        let task_result = sqlx::query(
            "UPDATE pipeline_tasks SET updated_at = NOW() WHERE task_id = $1 AND outcome IS NULL",
        )
        .bind(task_id)
        .execute(&mut *transaction)
        .await
        .map_err(PipelineTaskError::Lifecycle)?;
        if task_result.rows_affected() != 1 {
            return Err(PipelineTaskError::TransitionConflict { task_id, status });
        }
        transaction
            .commit()
            .await
            .map_err(PipelineTaskError::Lifecycle)
    }

    /// Resolves a moderation callback exclusively by its token digest before
    /// persisting its result.
    pub async fn complete_moderation_for_callback(
        &self,
        task_token: &str,
        job_id: i32,
        moderation_task_id: &str,
        result: ModerationResult,
    ) -> Result<(), PipelineTaskError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(PipelineTaskError::Lifecycle)?;
        let attempt = callback_attempt(&mut transaction, task_token).await?;
        if attempt.step != PipelineCallbackStep::Moderation {
            return Err(PipelineTaskError::CallbackStepConflict);
        }
        if attempt.task_id != job_id {
            return Err(PipelineTaskError::ModerationJobConflict);
        }
        transaction
            .commit()
            .await
            .map_err(PipelineTaskError::Lifecycle)?;
        self.complete_moderation(attempt.task_id, task_token, moderation_task_id, result)
            .await
    }

    /// Persists an ASR failure after resolving its task exclusively by the
    /// stored token digest. Parent finalization waits until Step Functions
    /// accepts the failure so a timed out token can be recorded as timed out.
    pub async fn record_asr_failure(
        &self,
        task_token: &str,
        error_code: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<i32, PipelineTaskError> {
        let attempt = self
            .record_callback_failure_for_step(
                task_token,
                error_code,
                error_message,
                Some(PipelineCallbackStep::Transcription),
            )
            .await?;
        Ok(attempt.task_id)
    }

    /// Persists a failure for the step identified by a callback token. The
    /// parent workflow is finalized separately after Step Functions accepts it.
    pub async fn record_callback_failure(
        &self,
        task_token: &str,
        error_code: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<CallbackAttempt, PipelineTaskError> {
        self.record_callback_failure_for_step(task_token, error_code, error_message, None)
            .await
    }

    async fn record_callback_failure_for_step(
        &self,
        task_token: &str,
        error_code: Option<&str>,
        error_message: Option<&str>,
        expected_step: Option<PipelineCallbackStep>,
    ) -> Result<CallbackAttempt, PipelineTaskError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(PipelineTaskError::Lifecycle)?;
        let attempt = callback_attempt(&mut transaction, task_token).await?;
        if expected_step.is_some_and(|step| step != attempt.step) {
            return Err(PipelineTaskError::CallbackStepConflict);
        }
        let status = task_status_for_update(&mut transaction, attempt.task_id).await?;
        match (attempt.step, status) {
            (PipelineCallbackStep::Transcription, PipelineTaskStatus::StartedAsr) => {
                sqlx::query(
                    "UPDATE transcription_tasks SET status = 'FAILED', error_code = $2, \
                     error_message = $3, completed_at = NOW(), updated_at = NOW() \
                     WHERE task_id = $1 AND status = 'PROCESSING'",
                )
                .bind(attempt.task_id)
                .bind(error_code)
                .bind(error_message)
                .execute(&mut *transaction)
                .await
                .map_err(PipelineTaskError::Lifecycle)?;
            }
            (PipelineCallbackStep::Transcription, PipelineTaskStatus::Failed) => {
                let existing = sqlx::query_as::<_, (Option<String>, Option<String>)>(
                    "SELECT error_code, error_message FROM transcription_tasks WHERE task_id = $1",
                )
                .bind(attempt.task_id)
                .fetch_one(&mut *transaction)
                .await
                .map_err(PipelineTaskError::Lifecycle)?;
                if existing
                    != (
                        error_code.map(str::to_owned),
                        error_message.map(str::to_owned),
                    )
                {
                    return Err(PipelineTaskError::TranscriptionConflict);
                }
            }
            (PipelineCallbackStep::Moderation, PipelineTaskStatus::StartedModerationProcessing) => {
                sqlx::query(
                    "UPDATE moderation_tasks SET status = 'FAILED', error_code = $2, \
                     error_message = $3, completed_at = NOW(), updated_at = NOW() \
                     WHERE task_id = $1 AND status = 'PROCESSING'",
                )
                .bind(attempt.task_id)
                .bind(error_code)
                .bind(error_message)
                .execute(&mut *transaction)
                .await
                .map_err(PipelineTaskError::Lifecycle)?;
            }
            (PipelineCallbackStep::Moderation, PipelineTaskStatus::Failed) => {
                let existing = sqlx::query_as::<_, (Option<String>, Option<String>)>(
                    "SELECT error_code, error_message FROM moderation_tasks WHERE task_id = $1",
                )
                .bind(attempt.task_id)
                .fetch_one(&mut *transaction)
                .await
                .map_err(PipelineTaskError::Lifecycle)?;
                if existing
                    != (
                        error_code.map(str::to_owned),
                        error_message.map(str::to_owned),
                    )
                {
                    return Err(PipelineTaskError::ModerationConflict);
                }
            }
            (_, status) => {
                return Err(PipelineTaskError::TransitionConflict {
                    task_id: attempt.task_id,
                    status,
                });
            }
        }
        transaction
            .commit()
            .await
            .map_err(PipelineTaskError::Lifecycle)?;
        Ok(attempt)
    }

    /// Records a caller-side ASR request failure. Unlike an external callback,
    /// this invocation is the workflow's final application failure, so it can
    /// transition the parent immediately.
    pub async fn fail_asr_request(
        &self,
        task_id: i32,
        error_code: &str,
        error_message: &str,
    ) -> Result<(), PipelineTaskError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(PipelineTaskError::Lifecycle)?;
        let status = task_status_for_update(&mut transaction, task_id).await?;
        match status {
            PipelineTaskStatus::StartedAsr => {
                sqlx::query(
                    "UPDATE transcription_tasks SET status = 'FAILED', error_code = $2, \
                     error_message = $3, completed_at = NOW(), updated_at = NOW() \
                     WHERE task_id = $1 AND status = 'PROCESSING'",
                )
                .bind(task_id)
                .bind(error_code)
                .bind(error_message)
                .execute(&mut *transaction)
                .await
                .map_err(PipelineTaskError::Lifecycle)?;
                sqlx::query(
                    "UPDATE pipeline_tasks SET outcome = 'FAILED', completed_at = NOW(), updated_at = NOW() \
                     WHERE task_id = $1 AND outcome IS NULL",
                )
                .bind(task_id)
                .execute(&mut *transaction)
                .await
                .map_err(PipelineTaskError::Lifecycle)?;
            }
            PipelineTaskStatus::Failed => {}
            status => return Err(PipelineTaskError::TransitionConflict { task_id, status }),
        }
        transaction
            .commit()
            .await
            .map_err(PipelineTaskError::Lifecycle)
    }

    /// Records a caller-side moderation request failure as terminal.
    pub async fn fail_moderation_request(
        &self,
        task_id: i32,
        error_code: &str,
        error_message: &str,
    ) -> Result<(), PipelineTaskError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(PipelineTaskError::Lifecycle)?;
        let status = task_status_for_update(&mut transaction, task_id).await?;
        match status {
            PipelineTaskStatus::StartedModerationProcessing => {
                sqlx::query(
                    "UPDATE moderation_tasks SET status = 'FAILED', error_code = $2, \
                     error_message = $3, completed_at = NOW(), updated_at = NOW() \
                     WHERE task_id = $1 AND status = 'PROCESSING'",
                )
                .bind(task_id)
                .bind(error_code)
                .bind(error_message)
                .execute(&mut *transaction)
                .await
                .map_err(PipelineTaskError::Lifecycle)?;
                sqlx::query(
                    "UPDATE pipeline_tasks SET outcome = 'FAILED', completed_at = NOW(), updated_at = NOW() \
                     WHERE task_id = $1 AND outcome IS NULL",
                )
                .bind(task_id)
                .execute(&mut *transaction)
                .await
                .map_err(PipelineTaskError::Lifecycle)?;
            }
            PipelineTaskStatus::Failed => {}
            status => return Err(PipelineTaskError::TransitionConflict { task_id, status }),
        }
        transaction
            .commit()
            .await
            .map_err(PipelineTaskError::Lifecycle)
    }

    /// Finalizes the legacy ASR-only workflow path using its callback token.
    pub async fn finalize_asr_success(
        &self,
        task_id: i32,
        task_token: &str,
    ) -> Result<(), PipelineTaskError> {
        self.finish_workflow_with_callback(
            task_id,
            PipelineTaskOutcome::Succeeded,
            Some((task_token, PipelineCallbackStep::Transcription)),
            None,
            None,
        )
        .await
    }

    pub async fn finalize_asr_failure(
        &self,
        task_id: i32,
        task_token: &str,
    ) -> Result<(), PipelineTaskError> {
        self.finish_workflow_with_callback(
            task_id,
            PipelineTaskOutcome::Failed,
            Some((task_token, PipelineCallbackStep::Transcription)),
            None,
            None,
        )
        .await
    }

    /// Finalizes the parent workflow after a failure callback has been
    /// accepted by Step Functions, resolving the step from the token.
    pub async fn finalize_callback_failure(
        &self,
        task_token: &str,
    ) -> Result<(), PipelineTaskError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(PipelineTaskError::Lifecycle)?;
        let attempt = callback_attempt(&mut transaction, task_token).await?;
        transaction
            .commit()
            .await
            .map_err(PipelineTaskError::Lifecycle)?;
        self.finish_workflow_with_callback(
            attempt.task_id,
            PipelineTaskOutcome::Failed,
            Some((task_token, attempt.step)),
            None,
            None,
        )
        .await
    }

    /// Records a terminal state delivered by workflow catch/reconciliation.
    pub async fn finish_workflow(
        &self,
        task_id: i32,
        outcome: PipelineTaskOutcome,
        task_token: Option<&str>,
        error_code: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<(), PipelineTaskError> {
        self.finish_workflow_with_callback(
            task_id,
            outcome,
            task_token.map(|token| (token, PipelineCallbackStep::Transcription)),
            error_code,
            error_message,
        )
        .await
    }

    async fn finish_workflow_with_callback(
        &self,
        task_id: i32,
        outcome: PipelineTaskOutcome,
        callback: Option<(&str, PipelineCallbackStep)>,
        error_code: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<(), PipelineTaskError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(PipelineTaskError::Lifecycle)?;
        if let Some((task_token, step)) = callback {
            validate_callback_token(&mut transaction, task_id, step, task_token).await?;
        }
        let status = task_status_for_update(&mut transaction, task_id).await?;
        if matches!(
            status,
            PipelineTaskStatus::Succeeded
                | PipelineTaskStatus::Failed
                | PipelineTaskStatus::TimedOut
                | PipelineTaskStatus::Cancelled
        ) {
            let existing = sqlx::query_scalar::<_, Option<PipelineTaskOutcome>>(
                "SELECT outcome FROM pipeline_tasks WHERE task_id = $1",
            )
            .bind(task_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(PipelineTaskError::Lifecycle)?;
            if let Some(existing) = existing {
                if existing == outcome {
                    return transaction
                        .commit()
                        .await
                        .map_err(PipelineTaskError::Lifecycle);
                }
                return Err(PipelineTaskError::TransitionConflict { task_id, status });
            }
        }
        if outcome == PipelineTaskOutcome::Succeeded
            && !matches!(
                status,
                PipelineTaskStatus::AsrFinished | PipelineTaskStatus::ModerationProcessingFinished
            )
        {
            return Err(PipelineTaskError::TransitionConflict { task_id, status });
        }
        if outcome != PipelineTaskOutcome::Succeeded {
            sqlx::query(
                "UPDATE audio_processing_tasks SET status = 'FAILED', error_code = $2, error_message = $3, \
                 completed_at = NOW(), updated_at = NOW() WHERE task_id = $1 AND status = 'PROCESSING'",
            )
            .bind(task_id)
            .bind(error_code.or(Some("WORKFLOW_TERMINATED")))
            .bind(error_message)
            .execute(&mut *transaction)
            .await
            .map_err(PipelineTaskError::Lifecycle)?;
            sqlx::query(
                "UPDATE transcription_tasks SET status = 'FAILED', error_code = $2, error_message = $3, \
                 completed_at = NOW(), updated_at = NOW() WHERE task_id = $1 AND status = 'PROCESSING'",
            )
            .bind(task_id)
            .bind(error_code.or(Some("WORKFLOW_TERMINATED")))
            .bind(error_message)
            .execute(&mut *transaction)
            .await
            .map_err(PipelineTaskError::Lifecycle)?;
            sqlx::query(
                "UPDATE moderation_tasks SET status = 'FAILED', error_code = $2, error_message = $3, \
                 completed_at = NOW(), updated_at = NOW() WHERE task_id = $1 AND status = 'PROCESSING'",
            )
            .bind(task_id)
            .bind(error_code.or(Some("WORKFLOW_TERMINATED")))
            .bind(error_message)
            .execute(&mut *transaction)
            .await
            .map_err(PipelineTaskError::Lifecycle)?;
        }
        sqlx::query(
            "UPDATE pipeline_tasks SET outcome = $2, completed_at = NOW(), updated_at = NOW() \
             WHERE task_id = $1 AND outcome IS NULL",
        )
        .bind(task_id)
        .bind(outcome)
        .execute(&mut *transaction)
        .await
        .map_err(PipelineTaskError::Lifecycle)?;
        transaction
            .commit()
            .await
            .map_err(PipelineTaskError::Lifecycle)
    }

    /// Atomically leases an undispatched task and returns its attempt number.
    ///
    /// An expired lease permits recovery after a Lambda exits while dispatching.
    /// Returning `None` means another invocation owns the lease, the task was
    /// already dispatched, or the retry limit has been reached.
    pub async fn claim_dispatch(&self, task_id: i32) -> Result<Option<i32>, PipelineTaskError> {
        let lease_seconds = i64::try_from(self.dispatch_lease.as_secs())
            .expect("dispatch lease duration must fit in PostgreSQL BIGINT seconds");
        let attempt_count = sqlx::query_scalar::<_, i32>(
            r#"
                UPDATE pipeline_tasks
                SET dispatch_started_at = NOW(),
                    attempt_count = attempt_count + 1,
                    updated_at = NOW()
                WHERE task_id = $1
                    AND execution_arn IS NULL
                    AND outcome IS NULL
                    AND attempt_count < 3
                    AND (
                        dispatch_started_at IS NULL
                        OR dispatch_started_at < NOW() - ($2 * INTERVAL '1 second')
                    )
                RETURNING attempt_count
            "#,
        )
        .bind(task_id)
        .bind(lease_seconds)
        .fetch_optional(&self.pool)
        .await
        .map_err(PipelineTaskError::ClaimDispatch)?;

        Ok(attempt_count)
    }

    /// Releases the dispatch lease after a failure so a later request can retry.
    /// The third failure moves the task to its terminal `FAILED` state.
    pub async fn record_dispatch_failure(&self, task_id: i32) -> Result<bool, PipelineTaskError> {
        let outcome = sqlx::query_scalar::<_, Option<PipelineTaskOutcome>>(
            r#"
                UPDATE pipeline_tasks
                SET dispatch_started_at = NULL,
                    outcome = CASE
                        WHEN attempt_count >= 3 THEN 'FAILED'::pipeline_task_outcome
                        ELSE outcome
                    END,
                    completed_at = CASE
                        WHEN attempt_count >= 3 THEN NOW()
                        ELSE completed_at
                    END,
                    updated_at = NOW()
                WHERE task_id = $1 AND execution_arn IS NULL AND outcome IS NULL
                RETURNING outcome
            "#,
        )
        .bind(task_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(PipelineTaskError::RecordDispatchFailure)?;

        Ok(outcome.flatten() == Some(PipelineTaskOutcome::Failed))
    }
}

fn task_token_hash(task_token: &str) -> String {
    format!("{:x}", Sha256::digest(task_token.as_bytes()))
}

async fn callback_attempt(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    task_token: &str,
) -> Result<CallbackAttempt, PipelineTaskError> {
    // Callback attempts are immutable after insertion, so a plain lookup
    // avoids taking their lock before the pipeline task lock.
    sqlx::query_as::<_, CallbackAttempt>(
        "SELECT task_id, step FROM pipeline_callback_attempts WHERE task_token_hash = $1",
    )
    .bind(task_token_hash(task_token))
    .fetch_optional(&mut **transaction)
    .await
    .map_err(PipelineTaskError::Lifecycle)?
    .ok_or(PipelineTaskError::CallbackTokenNotFound)
}

async fn validate_callback_token(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    task_id: i32,
    step: PipelineCallbackStep,
    task_token: &str,
) -> Result<(), PipelineTaskError> {
    let callback = callback_attempt(transaction, task_token).await?;
    if callback.task_id != task_id {
        return Err(PipelineTaskError::CallbackTaskConflict);
    }
    if callback.step != step {
        return Err(PipelineTaskError::CallbackStepConflict);
    }
    Ok(())
}

async fn insert_callback_attempt(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    task_id: i32,
    step: PipelineCallbackStep,
    token_hash: &str,
) -> Result<(), PipelineTaskError> {
    let insert = sqlx::query(
        "INSERT INTO pipeline_callback_attempts (task_id, step, task_token_hash) \
         VALUES ($1, $2, $3) ON CONFLICT (task_id, step, task_token_hash) DO NOTHING",
    )
    .bind(task_id)
    .bind(step)
    .bind(token_hash)
    .execute(&mut **transaction)
    .await
    .map_err(PipelineTaskError::Lifecycle)?;
    if insert.rows_affected() == 0 {
        let owner = sqlx::query_as::<_, CallbackAttempt>(
            "SELECT task_id, step FROM pipeline_callback_attempts WHERE task_token_hash = $1",
        )
        .bind(token_hash)
        .fetch_one(&mut **transaction)
        .await
        .map_err(PipelineTaskError::Lifecycle)?;
        if owner.task_id != task_id {
            return Err(PipelineTaskError::CallbackTaskConflict);
        }
        if owner.step != step {
            return Err(PipelineTaskError::CallbackStepConflict);
        }
    }
    Ok(())
}

async fn task_status_for_update(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    task_id: i32,
) -> Result<PipelineTaskStatus, PipelineTaskError> {
    let outcome = sqlx::query_scalar::<_, Option<PipelineTaskOutcome>>(
        "SELECT outcome FROM pipeline_tasks WHERE task_id = $1 FOR UPDATE",
    )
    .bind(task_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(PipelineTaskError::Lifecycle)?
    .ok_or(PipelineTaskError::TaskNotFound(task_id))?;

    // Read the steps after acquiring the pipeline lock so a concurrent
    // transition cannot leave this derivation with a stale step snapshot.
    let steps = sqlx::query_as::<_, PipelineStepState>(
        r#"
            SELECT audio.status AS audio_status, transcription.status AS transcription_status,
                moderation.status AS moderation_status
            FROM audio_processing_tasks audio
            JOIN transcription_tasks transcription USING (task_id)
            JOIN moderation_tasks moderation USING (task_id)
            WHERE audio.task_id = $1
        "#,
    )
    .bind(task_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(PipelineTaskError::Lifecycle)?;

    Ok(steps.status(outcome))
}
