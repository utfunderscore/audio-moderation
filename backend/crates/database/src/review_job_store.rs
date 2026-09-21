use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// Errors returned while accessing persisted review jobs.
#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    #[error("failed to create or retrieve review job")]
    CreateOrGet(#[source] sqlx::Error),

    #[error("failed to retrieve review job")]
    Get(#[source] sqlx::Error),

    #[error("failed to update review job status")]
    UpdateStatus(#[source] sqlx::Error),

    #[error("failed to mark review job upload complete")]
    MarkUploadComplete(#[source] sqlx::Error),
}

/// The persisted state for a review request.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct ReviewJob {
    pub job_id: i32,
    pub tenant_id: String,
    pub idempotency_key: Option<String>,
    pub access_token_hash: String,
    pub status: ReviewJobStatus,
    pub input_file_path: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created: bool,
}

/// Caller-visible review state and its asynchronously-created evaluation linkage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewJobDetails {
    pub job: ReviewJob,
    pub evaluation_id: Option<Uuid>,
}

/// Values required to create a review job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewReviewJob<'a> {
    pub tenant_id: &'a str,
    pub idempotency_key: Option<&'a str>,
    pub access_token_hash: &'a str,
}

/// Workflow states persisted in `review_jobs.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "review_job_status", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewJobStatus {
    AwaitingUpload,
    PendingProcessing,
    Processing,
    Completed,
    Error,
}

impl ReviewJobStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::AwaitingUpload => "AWAITING_UPLOAD",
            Self::PendingProcessing => "PENDING_PROCESSING",
            Self::Processing => "PROCESSING",
            Self::Completed => "COMPLETED",
            Self::Error => "ERROR",
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
    pub async fn create_or_get(&self, job: NewReviewJob<'_>) -> Result<ReviewJob, DatabaseError> {
        sqlx::query_as::<_, ReviewJob>(
            r#"
                INSERT INTO review_jobs (tenant_id, idempotency_key, access_token_hash)
                VALUES ($1, $2, $3)
                ON CONFLICT (tenant_id, idempotency_key) DO UPDATE
                    SET tenant_id = review_jobs.tenant_id
                RETURNING job_id, tenant_id, idempotency_key,
                    access_token_hash, status, input_file_path, created_at, updated_at,
                    (xmax = 0) AS created
            "#,
        )
        .bind(job.tenant_id)
        .bind(job.idempotency_key)
        .bind(job.access_token_hash)
        .fetch_one(&self.pool)
        .await
        .map_err(DatabaseError::CreateOrGet)
    }

    /// Gets a job only when it belongs to `tenant_id`.
    pub async fn get(
        &self,
        job_id: i32,
        tenant_id: &str,
    ) -> Result<Option<ReviewJob>, DatabaseError> {
        sqlx::query_as::<_, ReviewJob>(
            r#"
                SELECT job_id, tenant_id, idempotency_key,
                    access_token_hash, status, input_file_path, created_at, updated_at,
                    TRUE AS created
                FROM review_jobs
                WHERE job_id = $1 AND tenant_id = $2
            "#,
        )
        .bind(job_id)
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DatabaseError::Get)
    }

    /// Finds a review by its generated upload key within one tenant.
    pub async fn get_by_input_file_path(
        &self,
        input_file_path: &str,
        tenant_id: &str,
    ) -> Result<Option<ReviewJob>, DatabaseError> {
        sqlx::query_as::<_, ReviewJob>(
            r#"
                SELECT job_id, tenant_id, idempotency_key, access_token_hash,
                    status, input_file_path, created_at, updated_at, TRUE AS created
                FROM review_jobs
                WHERE input_file_path = $1 AND tenant_id = $2
            "#,
        )
        .bind(input_file_path)
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DatabaseError::Get)
    }

    /// Gets a tenant-scoped review and its explicitly linked pipeline evaluation.
    pub async fn get_details(
        &self,
        job_id: i32,
        tenant_id: &str,
    ) -> Result<Option<ReviewJobDetails>, DatabaseError> {
        #[derive(sqlx::FromRow)]
        struct Row {
            job_id: i32,
            tenant_id: String,
            idempotency_key: Option<String>,
            access_token_hash: String,
            status: ReviewJobStatus,
            input_file_path: String,
            created_at: DateTime<Utc>,
            updated_at: DateTime<Utc>,
            evaluation_id: Option<Uuid>,
        }

        let row = sqlx::query_as::<_, Row>(
            r#"
                SELECT review.job_id, review.tenant_id, review.idempotency_key, review.access_token_hash,
                    CASE
                        WHEN pipeline.outcome = 'SUCCEEDED' THEN 'COMPLETED'::review_job_status
                        WHEN pipeline.outcome IS NOT NULL THEN 'ERROR'::review_job_status
                        ELSE review.status
                    END AS status,
                    review.input_file_path, review.created_at,
                    GREATEST(review.updated_at, COALESCE(pipeline.updated_at, review.updated_at)) AS updated_at,
                    pipeline.evaluation_id
                FROM review_jobs review
                LEFT JOIN pipeline_tasks pipeline ON pipeline.review_job_id = review.job_id
                WHERE review.job_id = $1 AND review.tenant_id = $2
            "#,
        )
        .bind(job_id)
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DatabaseError::Get)?;

        Ok(row.map(|row| ReviewJobDetails {
            job: ReviewJob {
                job_id: row.job_id,
                tenant_id: row.tenant_id,
                idempotency_key: row.idempotency_key,
                access_token_hash: row.access_token_hash,
                status: row.status,
                input_file_path: row.input_file_path,
                created_at: row.created_at,
                updated_at: row.updated_at,
                created: false,
            },
            evaluation_id: row.evaluation_id,
        }))
    }

    /// Updates a job's workflow status and returns the updated row.
    pub async fn update_status(
        &self,
        job_id: i32,
        tenant_id: &str,
        status: ReviewJobStatus,
    ) -> Result<Option<ReviewJob>, DatabaseError> {
        sqlx::query_as::<_, ReviewJob>(
            r#"
                UPDATE review_jobs
                SET status = $3::text::review_job_status, updated_at = NOW()
                WHERE job_id = $1 AND tenant_id = $2
                RETURNING job_id, tenant_id, idempotency_key,
                    access_token_hash, status, input_file_path, created_at, updated_at,
                    TRUE AS created
            "#,
        )
        .bind(job_id)
        .bind(tenant_id)
        .bind(status.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(DatabaseError::UpdateStatus)
    }

    /// Advances the job for an uploaded source object to `PENDING_PROCESSING`.
    ///
    /// Repeated upload notifications return the job's current status without
    /// moving it backwards, making S3's at-least-once delivery safe.
    pub async fn mark_upload_complete(
        &self,
        input_file_path: &str,
        tenant_id: &str,
    ) -> Result<Option<ReviewJob>, DatabaseError> {
        sqlx::query_as::<_, ReviewJob>(
            r#"
                UPDATE review_jobs
                SET
                    status = CASE
                        WHEN status = 'AWAITING_UPLOAD' THEN 'PENDING_PROCESSING'::review_job_status
                        ELSE status
                    END,
                    updated_at = CASE
                        WHEN status = 'AWAITING_UPLOAD' THEN NOW()
                        ELSE updated_at
                    END
                WHERE input_file_path = $1 AND tenant_id = $2
                RETURNING job_id, tenant_id, idempotency_key,
                    access_token_hash, status, input_file_path, created_at, updated_at,
                    TRUE AS created
            "#,
        )
        .bind(input_file_path)
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DatabaseError::MarkUploadComplete)
    }

    /// Finds the review explicitly linked to an evaluation within one tenant.
    /// The caller compares its stored token digest before exposing any result.
    pub async fn get_by_evaluation_id(
        &self,
        evaluation_id: &Uuid,
        tenant_id: &str,
    ) -> Result<Option<ReviewJob>, DatabaseError> {
        sqlx::query_as::<_, ReviewJob>(
            r#"
                SELECT review.job_id, review.tenant_id, review.idempotency_key,
                    review.access_token_hash, review.status, review.input_file_path,
                    review.created_at, review.updated_at, TRUE AS created
                FROM review_jobs review
                INNER JOIN pipeline_tasks pipeline ON pipeline.review_job_id = review.job_id
                    AND pipeline.tenant_id = review.tenant_id
                WHERE pipeline.evaluation_id = $1 AND review.tenant_id = $2
            "#,
        )
        .bind(evaluation_id)
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DatabaseError::Get)
    }
}
