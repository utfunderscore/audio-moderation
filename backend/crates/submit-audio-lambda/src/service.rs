use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aws_sdk_s3::Client as S3Client;
use aws_sdk_s3::presigning::PresigningConfig;
use buffa_types::google::protobuf::Timestamp;
use common::EvaluationAccess;
use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest};
use database::{
    NewReviewPipelineTask, PipelineTaskEventTicketStore, PipelineTaskStore,
    ReviewJobStatus as DatabaseReviewJobStatus, ReviewJobStore,
};
use tracing::{error, info, warn};

use crate::proto::audio::review::v1::{
    AudioReviewService, CreateReviewEventsTicketRequest, CreateReviewEventsTicketResponse,
    GetReviewRequest, GetReviewResponse, Review, ReviewJobStatus, SubmitReviewRequest,
    SubmitReviewResponse,
};

const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";
const UPLOAD_URL_TTL: Duration = Duration::from_secs(15 * 60);
const REVIEW_ACCESS_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const AUTHORIZATION_HEADER: &str = "authorization";
const TASK_EVENTS_TICKET_LIFETIME: chrono::Duration = chrono::Duration::minutes(2);

#[derive(Clone)]
pub(crate) struct SubmitReviewService {
    store: ReviewJobStore,
    pipeline_store: PipelineTaskStore,
    tickets: PipelineTaskEventTicketStore,
    s3_client: S3Client,
    uploads_bucket: String,
    tenant_id: String,
    access: EvaluationAccess,
}

impl SubmitReviewService {
    pub(crate) fn new(
        store: ReviewJobStore,
        pipeline_store: PipelineTaskStore,
        tickets: PipelineTaskEventTicketStore,
        s3_client: S3Client,
        uploads_bucket: String,
        tenant_id: String,
        access: EvaluationAccess,
    ) -> Self {
        Self {
            store,
            pipeline_store,
            tickets,
            s3_client,
            uploads_bucket,
            tenant_id,
            access,
        }
    }

    fn capability(
        &self,
        job_id: i32,
        created_at: chrono::DateTime<chrono::Utc>,
    ) -> (String, Timestamp) {
        // Anchor expiry to creation, so an idempotent replay cannot extend the
        // capability's lifetime.
        let expires_at = SystemTime::from(created_at) + REVIEW_ACCESS_TTL;
        let expires_at = UNIX_EPOCH
            + Duration::from_secs(
                expires_at
                    .duration_since(UNIX_EPOCH)
                    .expect("review creation is after Unix epoch")
                    .as_secs(),
            );
        (
            self.access
                .review_token(&self.tenant_id, job_id, expires_at),
            system_timestamp(expires_at),
        )
    }

    fn authorize(&self, ctx: &RequestContext, review_id: i32) -> Result<(), ConnectError> {
        let value = ctx
            .header(AUTHORIZATION_HEADER)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| ConnectError::unauthenticated("review access token is required"))?;
        let (scheme, token) = value
            .trim()
            .split_once(' ')
            .ok_or_else(|| ConnectError::unauthenticated("invalid review access token"))?;
        if !scheme.eq_ignore_ascii_case("Bearer")
            || token.is_empty()
            || self
                .access
                .authenticated_review(&self.tenant_id, token, SystemTime::now())
                != Some(review_id)
        {
            return Err(ConnectError::unauthenticated("invalid review access token"));
        }
        Ok(())
    }
}

impl AudioReviewService for SubmitReviewService {
    async fn submit_review<'a>(
        &'a self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SubmitReviewRequest>,
    ) -> connectrpc::ServiceResult<impl connectrpc::Encodable<SubmitReviewResponse> + Send + use<'a>>
    {
        let content_type = validate_content_type(request.content_type).map_err(|error| {
            warn!(
                tenantId = self.tenant_id,
                outcome = "rejected",
                reason = ?error,
                "rejected audio submission"
            );
            error
        })?;
        let idempotency_key = required_header(&ctx, IDEMPOTENCY_KEY_HEADER).map_err(|error| {
            warn!(
                tenantId = self.tenant_id,
                outcome = "rejected",
                reason = ?error,
                "rejected audio submission"
            );
            error
        })?;

        let review = self
            .pipeline_store
            .create_or_get_review(NewReviewPipelineTask {
                tenant_id: &self.tenant_id,
                idempotency_key: &idempotency_key,
                uploads_bucket: &self.uploads_bucket,
            })
            .await
            .map_err(|database_error| {
                error!(
                    tenantId = self.tenant_id,
                    outcome = "failed",
                    error = ?database_error,
                    "failed to persist audio submission"
                );
                ConnectError::internal("failed to create review")
            })?;
        let job = review.job;
        let task = review.task;
        let replayed = !job.created;
        if job.status != DatabaseReviewJobStatus::AwaitingUpload {
            info!(
                jobId = %job.job_id,
                tenantId = self.tenant_id,
                status = ?job.status,
                outcome = "idempotent_replay",
                "returned existing audio submission"
            );
            return Response::ok(self.response_for_existing_job(&job, &task.evaluation_id));
        }

        if replayed {
            let source_exists = match self
                .s3_client
                .head_object()
                .bucket(&self.uploads_bucket)
                .key(&job.input_file_path)
                .send()
                .await
            {
                Ok(_) => true,
                Err(error)
                    if error
                        .as_service_error()
                        .is_some_and(|service_error| service_error.is_not_found()) =>
                {
                    false
                }
                Err(head_error) => {
                    error!(
                        jobId = %job.job_id,
                        tenantId = self.tenant_id,
                        outcome = "failed",
                        error = ?head_error,
                        "failed to check existing source audio"
                    );
                    return Err(ConnectError::internal(
                        "failed to check existing source audio",
                    ));
                }
            };

            if source_exists {
                info!(
                    jobId = %job.job_id,
                    tenantId = self.tenant_id,
                    status = ?job.status,
                    outcome = "source_already_uploaded",
                    "returned existing audio submission"
                );
                return Response::ok(self.response_for_existing_job(&job, &task.evaluation_id));
            }
        }

        let upload = presign_upload(
            &self.s3_client,
            &self.uploads_bucket,
            &job.input_file_path,
            content_type,
        )
        .await
        .map_err(|presign_error| {
            error!(
                jobId = %job.job_id,
                tenantId = self.tenant_id,
                outcome = "failed",
                error = ?presign_error,
                "failed to issue audio upload URL"
            );
            presign_error
        })?;

        let upload_headers = upload
            .headers()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect();

        info!(
            jobId = %job.job_id,
            tenantId = self.tenant_id,
            status = ?job.status,
            outcome = "upload_url_issued",
            replayed,
            "accepted audio submission"
        );

        let (access_token, access_expires_at) = self.capability(job.job_id, job.created_at);
        Response::ok(SubmitReviewResponse {
            task_id: job.job_id.to_string(),
            review_id: job.job_id.to_string(),
            evaluation_id: task.evaluation_id,
            upload_url: upload.uri().to_owned(),
            upload_headers,
            status: ReviewJobStatus::AwaitingUpload.into(),
            access_token,
            access_expires_at: access_expires_at.into(),
            ..Default::default()
        })
    }

    async fn get_review<'a>(
        &'a self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetReviewRequest>,
    ) -> connectrpc::ServiceResult<impl connectrpc::Encodable<GetReviewResponse> + Send + use<'a>>
    {
        let review_id = parse_review_id(request.review_id)?;
        self.authorize(&ctx, review_id)?;
        let details = self
            .store
            .get_details(review_id, &self.tenant_id)
            .await
            .map_err(|database_error| {
                error!(reviewId = review_id, error = ?database_error, "failed to retrieve review");
                ConnectError::internal("failed to retrieve review")
            })?
            .ok_or_else(|| ConnectError::not_found("review not found"))?;

        Response::ok(GetReviewResponse {
            review: Review {
                review_id: details.job.job_id.to_string(),
                status: ReviewJobStatus::from(details.job.status).into(),
                evaluation_id: details
                    .evaluation_id
                    .map(|id| id.to_string())
                    .unwrap_or_default(),
                created_at: timestamp(details.job.created_at).into(),
                updated_at: timestamp(details.job.updated_at).into(),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        })
    }

    async fn create_review_events_ticket<'a>(
        &'a self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateReviewEventsTicketRequest>,
    ) -> connectrpc::ServiceResult<
        impl connectrpc::Encodable<CreateReviewEventsTicketResponse> + Send + use<'a>,
    > {
        let review_id = parse_review_id(request.review_id)?;
        self.authorize(&ctx, review_id)?;
        let task = self
            .pipeline_store
            .get_by_review_job(review_id, &self.tenant_id)
            .await
            .map_err(|database_error| {
                error!(reviewId = review_id, error = ?database_error, "failed to resolve review pipeline task");
                ConnectError::internal("failed to create review events ticket")
            })?
            .ok_or_else(|| ConnectError::internal("review is missing its pipeline task"))?;
        if task.review_job_id != Some(review_id) {
            return Err(ConnectError::internal(
                "review has invalid pipeline task linkage",
            ));
        }
        let ticket = format!("wst_v1.{}", uuid::Uuid::new_v4().simple());
        let persisted = self
            .tickets
            .create(task.task_id, &ticket, TASK_EVENTS_TICKET_LIFETIME)
            .await
            .map_err(|database_error| {
                error!(reviewId = review_id, error = ?database_error, "failed to create review events ticket");
                ConnectError::internal("failed to create review events ticket")
            })?;
        Response::ok(CreateReviewEventsTicketResponse {
            ticket,
            expires_at: timestamp(persisted.expires_at).into(),
            ..Default::default()
        })
    }
}

impl SubmitReviewService {
    fn response_for_existing_job(
        &self,
        job: &database::ReviewJob,
        evaluation_id: &str,
    ) -> SubmitReviewResponse {
        let (access_token, access_expires_at) = self.capability(job.job_id, job.created_at);
        SubmitReviewResponse {
            task_id: job.job_id.to_string(),
            review_id: job.job_id.to_string(),
            evaluation_id: evaluation_id.to_owned(),
            status: ReviewJobStatus::from(job.status).into(),
            access_token,
            access_expires_at: access_expires_at.into(),
            ..Default::default()
        }
    }
}

fn parse_review_id(value: &str) -> Result<i32, ConnectError> {
    value
        .trim()
        .parse()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| ConnectError::invalid_argument("review_id must be a positive integer"))
}

fn timestamp(value: chrono::DateTime<chrono::Utc>) -> Timestamp {
    Timestamp {
        seconds: value.timestamp(),
        nanos: value.timestamp_subsec_nanos() as i32,
        ..Default::default()
    }
}

fn system_timestamp(value: SystemTime) -> Timestamp {
    let value = value
        .duration_since(UNIX_EPOCH)
        .expect("expiry is after Unix epoch");
    Timestamp {
        seconds: value.as_secs() as i64,
        nanos: value.subsec_nanos() as i32,
        ..Default::default()
    }
}

impl From<DatabaseReviewJobStatus> for ReviewJobStatus {
    fn from(status: DatabaseReviewJobStatus) -> Self {
        match status {
            DatabaseReviewJobStatus::AwaitingUpload => Self::AwaitingUpload,
            DatabaseReviewJobStatus::PendingProcessing => Self::PendingProcessing,
            DatabaseReviewJobStatus::Processing => Self::Processing,
            DatabaseReviewJobStatus::Completed => Self::Completed,
            DatabaseReviewJobStatus::Error => Self::Error,
        }
    }
}

async fn presign_upload(
    s3_client: &S3Client,
    bucket: &str,
    key: &str,
    content_type: &str,
) -> Result<aws_sdk_s3::presigning::PresignedRequest, ConnectError> {
    let presigning_config = PresigningConfig::expires_in(UPLOAD_URL_TTL)
        .map_err(|_| ConnectError::internal("failed to configure upload URL"))?;

    s3_client
        .put_object()
        .bucket(bucket)
        .key(key)
        .content_type(content_type)
        .presigned(presigning_config)
        .await
        .map_err(|_| ConnectError::internal("failed to create upload URL"))
}

fn validate_content_type(content_type: &str) -> Result<&str, ConnectError> {
    let content_type = content_type.trim();
    if content_type.is_empty() {
        return Err(ConnectError::invalid_argument("content_type is required"));
    }
    if !content_type.starts_with("audio/") {
        return Err(ConnectError::invalid_argument(
            "content_type must be an audio media type",
        ));
    }
    Ok(content_type)
}

fn required_header(ctx: &RequestContext, name: &'static str) -> Result<String, ConnectError> {
    let value = ctx.header(name).ok_or_else(|| {
        ConnectError::invalid_argument(format!("{name} request header is required"))
    })?;
    let value = value
        .to_str()
        .map(str::trim)
        .map_err(|_| ConnectError::invalid_argument(format!("{name} is not valid ASCII")))?;
    if value.is_empty() {
        return Err(ConnectError::invalid_argument(format!(
            "{name} must not be empty"
        )));
    }
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_sdk_s3::config::{Credentials, Region};
    use connectrpc::ErrorCode;
    use lambda_http::http::{HeaderMap, HeaderValue};

    fn assert_invalid_argument<T: std::fmt::Debug>(
        result: Result<T, ConnectError>,
        expected_message: &str,
    ) {
        let error = result.unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidArgument);
        assert_eq!(error.message.as_deref(), Some(expected_message));
    }

    #[test]
    fn validates_content_type() {
        assert_eq!(validate_content_type(" audio/wav ").unwrap(), "audio/wav");
        assert_invalid_argument(validate_content_type(""), "content_type is required");
        assert_invalid_argument(
            validate_content_type("video/mp4"),
            "content_type must be an audio media type",
        );
    }

    #[test]
    fn reads_idempotency_key_case_insensitively() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "Idempotency-Key",
            HeaderValue::from_static(" submission-1 "),
        );
        let ctx = RequestContext::new(headers);

        assert_eq!(
            required_header(&ctx, IDEMPOTENCY_KEY_HEADER).unwrap(),
            "submission-1"
        );
    }

    #[test]
    fn rejects_invalid_idempotency_key_headers() {
        assert_invalid_argument(
            required_header(
                &RequestContext::new(HeaderMap::new()),
                IDEMPOTENCY_KEY_HEADER,
            ),
            "idempotency-key request header is required",
        );

        let mut headers = HeaderMap::new();
        headers.insert(IDEMPOTENCY_KEY_HEADER, HeaderValue::from_static("  "));
        assert_invalid_argument(
            required_header(&RequestContext::new(headers), IDEMPOTENCY_KEY_HEADER),
            "idempotency-key must not be empty",
        );

        let mut headers = HeaderMap::new();
        headers.insert(
            IDEMPOTENCY_KEY_HEADER,
            HeaderValue::from_bytes(b"\xff").unwrap(),
        );
        assert_invalid_argument(
            required_header(&RequestContext::new(headers), IDEMPOTENCY_KEY_HEADER),
            "idempotency-key is not valid ASCII",
        );
    }

    #[tokio::test]
    async fn replay_after_upload_returns_existing_job_without_upload_instructions() {
        let job_id = 42;
        let service = SubmitReviewService {
            store: ReviewJobStore::new(
                sqlx::postgres::PgPoolOptions::new()
                    .connect_lazy("postgres://localhost/test")
                    .unwrap(),
            ),
            pipeline_store: PipelineTaskStore::new(
                sqlx::postgres::PgPoolOptions::new()
                    .connect_lazy("postgres://localhost/test")
                    .unwrap(),
            ),
            tickets: PipelineTaskEventTicketStore::new(
                sqlx::postgres::PgPoolOptions::new()
                    .connect_lazy("postgres://localhost/test")
                    .unwrap(),
            ),
            s3_client: S3Client::from_conf(
                aws_sdk_s3::Config::builder()
                    .behavior_version_latest()
                    .build(),
            ),
            uploads_bucket: "unused".into(),
            tenant_id: "tenant-a".into(),
            access: EvaluationAccess::new("a sufficiently long test secret value".into()).unwrap(),
        };
        let job = database::ReviewJob {
            job_id,
            tenant_id: "tenant-a".into(),
            idempotency_key: Some("request-1".into()),
            status: DatabaseReviewJobStatus::Completed,
            input_file_path: "reviews/42/source".into(),
            created_at: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
            updated_at: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
            created: false,
        };

        let response = service.response_for_existing_job(&job, "evaluation-id");

        assert_eq!(response.task_id, job_id.to_string());
        assert_eq!(response.review_id, job_id.to_string());
        assert_eq!(response.evaluation_id, "evaluation-id");
        assert_eq!(response.status, ReviewJobStatus::Completed);
        assert!(response.access_token.starts_with("review_v1."));
        assert!(response.upload_url.is_empty());
        assert!(response.upload_headers.is_empty());
    }

    #[tokio::test]
    async fn presigns_put_for_the_requested_object_and_content_type() {
        let config = aws_sdk_s3::Config::builder()
            .behavior_version_latest()
            .credentials_provider(Credentials::new(
                "test-access-key",
                "test-secret-key",
                None,
                None,
                "test",
            ))
            .region(Region::new("eu-west-2"))
            .build();
        let client = S3Client::from_conf(config);

        let upload = presign_upload(
            &client,
            "uploads-bucket",
            "reviews/task-1/source",
            "audio/wav",
        )
        .await
        .unwrap();

        assert_eq!(upload.method(), "PUT");
        assert!(upload.uri().starts_with(
            "https://uploads-bucket.s3.eu-west-2.amazonaws.com/reviews/task-1/source?"
        ));
        assert!(upload.uri().contains("X-Amz-Expires=900"));
        assert!(
            upload
                .headers()
                .any(|(name, value)| name == "content-type" && value == "audio/wav")
        );
        assert!(!upload.headers().any(|(name, _)| name == "if-none-match"));
    }
}
