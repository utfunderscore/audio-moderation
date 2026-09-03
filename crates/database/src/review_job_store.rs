use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// The persisted state for a review request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewJob {
    pub job_id: Uuid,
    pub tenant_id: String,
    pub idempotency_key: Option<String>,
    pub status: ReviewJobStatus,
    pub input_file_path: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Values required to create a review job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewReviewJob<'a> {
    pub job_id: Uuid,
    pub tenant_id: &'a str,
    pub idempotency_key: Option<&'a str>,
    pub input_file_path: &'a str,
}

/// Workflow states persisted in `review_jobs.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "review_job_status", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewJobStatus {
    AwaitingUpload,
    Queued,
    StartedPreprocessingAudio,
    FinishedPreprocessingAudio,
    StartedTranscribing,
    FinishedTranscribing,
    StartedEvaluating,
    FinishedEvaluating,
    StartedPersistingResult,
    Completed,
    Failed,
}

impl ReviewJobStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::AwaitingUpload => "AWAITING_UPLOAD",
            Self::Queued => "QUEUED",
            Self::StartedPreprocessingAudio => "STARTED_PREPROCESSING_AUDIO",
            Self::FinishedPreprocessingAudio => "FINISHED_PREPROCESSING_AUDIO",
            Self::StartedTranscribing => "STARTED_TRANSCRIBING",
            Self::FinishedTranscribing => "FINISHED_TRANSCRIBING",
            Self::StartedEvaluating => "STARTED_EVALUATING",
            Self::FinishedEvaluating => "FINISHED_EVALUATING",
            Self::StartedPersistingResult => "STARTED_PERSISTING_RESULT",
            Self::Completed => "COMPLETED",
            Self::Failed => "FAILED",
        }
    }
}

/// SQLx-backed access to the `review_jobs` table.
#[derive(Clone)]
pub struct ReviewJobStore {
    pool: PgPool,
}

impl ReviewJobStore {
    /// Creates a store using the application's PostgreSQL connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Creates a job, or returns the existing job for the same tenant and idempotency key.
    pub async fn create_or_get(&self, job: NewReviewJob<'_>) -> Result<ReviewJob, sqlx::Error> {
        sqlx::query_as!(
            ReviewJob,
            r#"
                INSERT INTO review_jobs (job_id, tenant_id, idempotency_key, input_file_path)
                VALUES ($1, $2, $3, $4)
                ON CONFLICT (tenant_id, idempotency_key) DO UPDATE
                    SET tenant_id = review_jobs.tenant_id
                RETURNING job_id, tenant_id, idempotency_key,
                    status AS "status: ReviewJobStatus", input_file_path, created_at, updated_at
            "#,
            job.job_id,
            job.tenant_id,
            job.idempotency_key,
            job.input_file_path,
        )
        .fetch_one(&self.pool)
        .await
    }

    /// Gets a job only when it belongs to `tenant_id`.
    pub async fn get(
        &self,
        job_id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<ReviewJob>, sqlx::Error> {
        sqlx::query_as!(
            ReviewJob,
            r#"
                SELECT job_id, tenant_id, idempotency_key,
                    status AS "status: ReviewJobStatus", input_file_path, created_at, updated_at
                FROM review_jobs
                WHERE job_id = $1 AND tenant_id = $2
            "#,
            job_id,
            tenant_id,
        )
        .fetch_optional(&self.pool)
        .await
    }

    /// Updates a job's workflow status and returns the updated row.
    pub async fn update_status(
        &self,
        job_id: Uuid,
        tenant_id: &str,
        status: ReviewJobStatus,
    ) -> Result<Option<ReviewJob>, sqlx::Error> {
        sqlx::query_as!(
            ReviewJob,
            r#"
                UPDATE review_jobs
                SET status = $3::text::review_job_status, updated_at = NOW()
                WHERE job_id = $1 AND tenant_id = $2
                RETURNING job_id, tenant_id, idempotency_key,
                    status AS "status: ReviewJobStatus", input_file_path, created_at, updated_at
            "#,
            job_id,
            tenant_id,
            status.as_str(),
        )
        .fetch_optional(&self.pool)
        .await
    }
}
