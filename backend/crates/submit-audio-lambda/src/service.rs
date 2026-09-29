use std::sync::Arc;
use std::time::Duration;

use crate::turnstile::{MAX_TOKEN_BYTES, TurnstileVerifier, VerifyError};

#[cfg(test)]
#[path = "../../database/tests/common/mod.rs"]
mod test_database;
use aws_sdk_s3::Client as S3Client;
use aws_sdk_s3::presigning::PresigningConfig;
use buffa_types::google::protobuf::Timestamp;
use common::{review_token_hash, review_token_hash_matches};
use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest};
use database::{
    NewReviewPipelineTask, PipelineTask, PipelineTaskEventTicketStore, PipelineTaskStore,
    ReviewJob, ReviewJobStatus as DatabaseReviewJobStatus, ReviewJobStore,
};
use tracing::{error, info, warn};

use crate::proto::audio::review::v1::{
    AudioReviewService, CreateReviewEventsTicketRequest, CreateReviewEventsTicketResponse,
    GetReviewAudioRequest, GetReviewAudioResponse, GetReviewRequest, GetReviewResponse, Review,
    ReviewJobStatus, SubmitReviewRequest, SubmitReviewResponse,
};

const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";
const UPLOAD_URL_TTL: Duration = Duration::from_secs(15 * 60);
const DOWNLOAD_URL_TTL: Duration = Duration::from_secs(15 * 60);
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
    turnstile: Arc<dyn TurnstileVerifier>,
}

impl SubmitReviewService {
    pub(crate) fn new(
        store: ReviewJobStore,
        pipeline_store: PipelineTaskStore,
        tickets: PipelineTaskEventTicketStore,
        s3_client: S3Client,
        uploads_bucket: String,
        tenant_id: String,
        turnstile: impl TurnstileVerifier + 'static,
    ) -> Self {
        Self {
            store,
            pipeline_store,
            tickets,
            s3_client,
            uploads_bucket,
            tenant_id,
            turnstile: Arc::new(turnstile),
        }
    }

    async fn authorize(
        &self,
        ctx: &RequestContext,
        review_id: i32,
    ) -> Result<ReviewJob, ConnectError> {
        let token_hash = review_token_hash_from_authorization(ctx)?;
        let review = self.store.get(review_id, &self.tenant_id).await.map_err(
            |database_error| {
                error!(reviewId = review_id, error = ?database_error, "failed to authorize review");
                ConnectError::internal("failed to authorize review")
            },
        )?;
        let review =
            review.ok_or_else(|| ConnectError::unauthenticated("invalid review access token"))?;
        ensure_review_token_matches(&review.access_token_hash, &token_hash)?;
        Ok(review)
    }

    /// Only authenticated replays may bypass single-use Siteverify. This lookup
    /// does not touch the review row or reserve an idempotency key.
    async fn existing_submission(
        &self,
        idempotency_key: &str,
        access_token_hash: &str,
    ) -> Result<Option<(ReviewJob, PipelineTask)>, ConnectError> {
        let job = self
            .store
            .get_by_idempotency_key(&self.tenant_id, idempotency_key)
            .await
            .map_err(|database_error| {
                error!(error = ?database_error, "failed to look up audio submission");
                ConnectError::internal("failed to look up review")
            })?;
        let Some(job) = job else {
            return Ok(None);
        };
        ensure_review_token_matches(&job.access_token_hash, access_token_hash)?;
        let task = self
            .pipeline_store
            .get_by_review_job(job.job_id, &self.tenant_id)
            .await
            .map_err(|database_error| {
                error!(error = ?database_error, "failed to look up review pipeline task");
                ConnectError::internal("failed to look up review")
            })?
            .filter(|task| task.review_job_id == Some(job.job_id))
            .ok_or_else(|| ConnectError::internal("review is missing its pipeline task"))?;
        Ok(Some((job, task)))
    }

    async fn submission_response(
        &self,
        job: ReviewJob,
        task: PipelineTask,
        content_type: &str,
    ) -> connectrpc::ServiceResult<SubmitReviewResponse> {
        let replayed = !job.created;
        if job.status != DatabaseReviewJobStatus::AwaitingUpload {
            info!(jobId = %job.job_id, tenantId = self.tenant_id, status = ?job.status,
                outcome = "idempotent_replay", "returned existing audio submission");
            return Response::ok(Self::response_for_existing_job(&job, &task.evaluation_id));
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
                    error!(jobId = %job.job_id, tenantId = self.tenant_id,
                        outcome = "failed", error = ?head_error, "failed to check existing source audio");
                    return Err(ConnectError::internal(
                        "failed to check existing source audio",
                    ));
                }
            };
            if source_exists {
                info!(jobId = %job.job_id, tenantId = self.tenant_id, status = ?job.status,
                    outcome = "source_already_uploaded", "returned existing audio submission");
                return Response::ok(Self::response_for_existing_job(&job, &task.evaluation_id));
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
            error!(jobId = %job.job_id, tenantId = self.tenant_id,
                outcome = "failed", error = ?presign_error, "failed to issue audio upload URL");
            presign_error
        })?;
        let upload_headers = upload
            .headers()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect();
        info!(jobId = %job.job_id, tenantId = self.tenant_id, status = ?job.status,
            outcome = "upload_url_issued", replayed, "accepted audio submission");
        Response::ok(SubmitReviewResponse {
            task_id: job.job_id.to_string(),
            review_id: job.job_id.to_string(),
            evaluation_id: task.evaluation_id,
            upload_url: upload.uri().to_owned(),
            upload_headers,
            status: ReviewJobStatus::AwaitingUpload.into(),
            ..Default::default()
        })
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
        validate_idempotency_key(&idempotency_key)?;
        let access_token_hash = review_token_hash_from_authorization(&ctx)?;

        if let Some((job, task)) = self
            .existing_submission(&idempotency_key, &access_token_hash)
            .await?
        {
            return self.submission_response(job, task, content_type).await;
        }

        validate_turnstile_token(&request.turnstile_token)?;
        // This is deliberately unrelated to the client idempotency key. Siteverify
        // has no HTTP retry loop today, so one fresh UUID identifies this one
        // verification attempt. If retries are added, retain this UUID for them.
        let siteverify_key = uuid::Uuid::new_v4().to_string();
        if let Err(verification_error) = self
            .turnstile
            .verify(&request.turnstile_token, &siteverify_key)
            .await
        {
            // Another invocation may have completed verification and creation while
            // this one was verifying the same single-use token. Only its rightful
            // bearer may replay it; a miss remains a hard rejection, never a write.
            if let Some((job, task)) = self
                .existing_submission(&idempotency_key, &access_token_hash)
                .await?
            {
                return self.submission_response(job, task, content_type).await;
            }
            return Err(match verification_error {
                VerifyError::Rejected => {
                    ConnectError::permission_denied("Turnstile verification failed")
                }
                VerifyError::Unavailable => {
                    ConnectError::unavailable("Turnstile verification unavailable")
                }
            });
        }

        let review = self
            .pipeline_store
            .create_or_get_review(NewReviewPipelineTask {
                tenant_id: &self.tenant_id,
                idempotency_key: &idempotency_key,
                access_token_hash: &access_token_hash,
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
        // Do not reveal whether the idempotency key or review exists.
        ensure_review_token_matches(&review.job.access_token_hash, &access_token_hash)?;
        self.submission_response(review.job, review.task, content_type)
            .await
    }

    async fn get_review<'a>(
        &'a self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetReviewRequest>,
    ) -> connectrpc::ServiceResult<impl connectrpc::Encodable<GetReviewResponse> + Send + use<'a>>
    {
        let review_id = parse_review_id(request.review_id)?;
        self.authorize(&ctx, review_id).await?;
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

    async fn get_review_audio<'a>(
        &'a self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetReviewAudioRequest>,
    ) -> connectrpc::ServiceResult<
        impl connectrpc::Encodable<GetReviewAudioResponse> + Send + use<'a>,
    > {
        let review_id = parse_review_id(request.review_id)?;
        let review = self.authorize(&ctx, review_id).await?;
        let download = presign_download(
            &self.s3_client,
            &self.uploads_bucket,
            &review.input_file_path,
        )
        .await
        .map_err(|presign_error| {
            error!(
                reviewId = review_id,
                error = ?presign_error,
                "failed to issue audio download URL"
            );
            presign_error
        })?;
        let expires_at = chrono::Utc::now()
            + chrono::Duration::from_std(DOWNLOAD_URL_TTL)
                .map_err(|_| ConnectError::internal("failed to configure download URL"))?;

        Response::ok(GetReviewAudioResponse {
            download_url: download.uri().to_owned(),
            expires_at: timestamp(expires_at).into(),
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
        self.authorize(&ctx, review_id).await?;
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
        job: &database::ReviewJob,
        evaluation_id: &str,
    ) -> SubmitReviewResponse {
        SubmitReviewResponse {
            task_id: job.job_id.to_string(),
            review_id: job.job_id.to_string(),
            evaluation_id: evaluation_id.to_owned(),
            status: ReviewJobStatus::from(job.status).into(),
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

async fn presign_download(
    s3_client: &S3Client,
    bucket: &str,
    key: &str,
) -> Result<aws_sdk_s3::presigning::PresignedRequest, ConnectError> {
    let presigning_config = PresigningConfig::expires_in(DOWNLOAD_URL_TTL)
        .map_err(|_| ConnectError::internal("failed to configure download URL"))?;

    s3_client
        .get_object()
        .bucket(bucket)
        .key(key)
        .presigned(presigning_config)
        .await
        .map_err(|_| ConnectError::internal("failed to create download URL"))
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

fn review_token_hash_from_authorization(ctx: &RequestContext) -> Result<String, ConnectError> {
    let value = ctx
        .header(AUTHORIZATION_HEADER)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ConnectError::unauthenticated("invalid review access token"))?;
    let (scheme, token) = value
        .trim()
        .split_once(' ')
        .ok_or_else(|| ConnectError::unauthenticated("invalid review access token"))?;
    if !scheme.eq_ignore_ascii_case("Bearer") {
        return Err(ConnectError::unauthenticated("invalid review access token"));
    }
    review_token_hash(token)
        .ok_or_else(|| ConnectError::unauthenticated("invalid review access token"))
}

fn validate_idempotency_key(value: &str) -> Result<(), ConnectError> {
    uuid::Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| ConnectError::invalid_argument("idempotency-key must be a UUID"))
}

fn validate_turnstile_token(token: &str) -> Result<(), ConnectError> {
    if token.trim().is_empty() || token.len() > MAX_TOKEN_BYTES {
        return Err(ConnectError::invalid_argument("invalid Turnstile token"));
    }
    Ok(())
}

fn ensure_review_token_matches(expected: &str, supplied: &str) -> Result<(), ConnectError> {
    review_token_hash_matches(expected, supplied)
        .then_some(())
        .ok_or_else(|| ConnectError::unauthenticated("invalid review access token"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_sdk_s3::config::{Credentials, Region};
    use buffa::Message;
    use buffa::view::MessageView;
    use bytes::Bytes;
    use connectrpc::ErrorCode;
    use lambda_http::http::{HeaderMap, HeaderValue};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::test_database;

    #[derive(Clone)]
    struct FakeVerifier {
        calls: Arc<AtomicUsize>,
        result: Result<(), VerifyError>,
        verification_keys: Arc<Mutex<Vec<String>>>,
    }

    impl TurnstileVerifier for FakeVerifier {
        fn verify<'a>(
            &'a self,
            _: &'a str,
            verification_key: &'a str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), VerifyError>> + Send + 'a>>
        {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.verification_keys
                .lock()
                .unwrap()
                .push(verification_key.to_owned());
            Box::pin(std::future::ready(self.result))
        }
    }

    fn test_service(pool: sqlx::PgPool, verifier: FakeVerifier) -> SubmitReviewService {
        let config = aws_sdk_s3::Config::builder()
            .behavior_version_latest()
            .credentials_provider(Credentials::new(
                "test-key",
                "test-secret",
                None,
                None,
                "test",
            ))
            .region(Region::new("eu-west-2"))
            .build();
        SubmitReviewService::new(
            ReviewJobStore::new(pool.clone()),
            PipelineTaskStore::new(pool.clone()),
            PipelineTaskEventTicketStore::new(pool),
            S3Client::from_conf(config),
            "uploads".into(),
            "tenant-a".into(),
            verifier,
        )
    }

    async fn submit_for_test(
        service: &SubmitReviewService,
        key: &str,
        bearer: &str,
        token: &str,
    ) -> Result<(), ConnectError> {
        let mut headers = HeaderMap::new();
        headers.insert(IDEMPOTENCY_KEY_HEADER, HeaderValue::from_str(key).unwrap());
        headers.insert(
            AUTHORIZATION_HEADER,
            HeaderValue::from_str(&format!("Bearer {bearer}")).unwrap(),
        );
        let body = Bytes::from(
            SubmitReviewRequest {
                content_type: "audio/wav".into(),
                turnstile_token: token.into(),
                ..Default::default()
            }
            .encode_to_vec(),
        );
        let view =
            crate::proto::audio::review::v1::SubmitReviewRequestView::decode_view(&body).unwrap();
        service
            .submit_review(
                RequestContext::new(headers),
                ServiceRequest::from_parts(&view, &body),
            )
            .await
            .map(|_| ())
    }

    #[tokio::test]
    async fn denies_invalid_tokens_without_writing_and_authenticates_replays() {
        let db = test_database::TestDatabase::start().await;
        let calls = Arc::new(AtomicUsize::new(0));
        let verifier = FakeVerifier {
            calls: calls.clone(),
            result: Err(VerifyError::Rejected),
            verification_keys: Arc::default(),
        };
        let service = test_service(db.pool.clone(), verifier);
        // This valid noncanonical UUID must remain distinct from Siteverify's
        // backend-generated verification id.
        let key = "550E8400-E29B-41D4-A716-446655440000";
        let bearer = "review_v1.BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc";
        for token in ["", " ", &"x".repeat(MAX_TOKEN_BYTES + 1)] {
            let error = submit_for_test(&service, &key, bearer, token)
                .await
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidArgument);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let error = submit_for_test(&service, &key, bearer, "invalid-token")
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::PermissionDenied);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let unavailable = test_service(
            db.pool.clone(),
            FakeVerifier {
                calls: calls.clone(),
                result: Err(VerifyError::Unavailable),
                verification_keys: Arc::default(),
            },
        );
        let error = submit_for_test(&unavailable, &key, bearer, "valid-token")
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Unavailable);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(
            service
                .store
                .get_by_idempotency_key("tenant-a", &key)
                .await
                .unwrap()
                .is_none()
        );
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pipeline_tasks")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(count, 0);

        let verification_keys: Arc<Mutex<Vec<String>>> = Arc::default();
        let good = test_service(
            db.pool.clone(),
            FakeVerifier {
                calls: calls.clone(),
                result: Ok(()),
                verification_keys: verification_keys.clone(),
            },
        );
        submit_for_test(&good, &key, bearer, "valid-token")
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        let verification_key = verification_keys.lock().unwrap()[0].clone();
        assert_ne!(verification_key, key);
        assert_ne!(
            verification_key,
            uuid::Uuid::parse_str(key).unwrap().to_string(),
            "Siteverify must not canonicalize and reuse the client UUID"
        );
        assert_eq!(
            uuid::Uuid::parse_str(&verification_key)
                .unwrap()
                .get_version(),
            Some(uuid::Version::Random)
        );
        let persisted = good
            .store
            .get_by_idempotency_key("tenant-a", &key)
            .await
            .unwrap()
            .unwrap();
        good.store
            .update_status(
                persisted.job_id,
                "tenant-a",
                DatabaseReviewJobStatus::Processing,
            )
            .await
            .unwrap();
        // The same key and rightful bearer replays without a Turnstile token.
        submit_for_test(&service, &key, bearer, "").await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        let wrong_bearer = format!("review_v1.{}", {
            use base64::Engine;
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([8u8; 32])
        });
        // An invalid owner is rejected before its missing token is considered.
        let error = submit_for_test(&service, &key, &wrong_bearer, "")
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Unauthenticated);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM review_jobs")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

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
        assert_invalid_argument(
            validate_idempotency_key("not-a-uuid"),
            "idempotency-key must be a UUID",
        );
    }

    #[test]
    fn requires_a_strict_opaque_review_bearer_token() {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION_HEADER,
            HeaderValue::from_static(
                "Bearer review_v1.BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc",
            ),
        );
        assert!(review_token_hash_from_authorization(&RequestContext::new(headers)).is_ok());

        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION_HEADER,
            HeaderValue::from_static("Bearer review_v1.short"),
        );
        let error =
            review_token_hash_from_authorization(&RequestContext::new(headers)).unwrap_err();
        assert_eq!(error.code, ErrorCode::Unauthenticated);
        assert_eq!(
            error.message.as_deref(),
            Some("invalid review access token")
        );
    }

    #[test]
    fn mismatched_review_token_has_a_generic_non_disclosing_error() {
        let error = ensure_review_token_matches(&"a".repeat(64), &"b".repeat(64)).unwrap_err();
        assert_eq!(error.code, ErrorCode::Unauthenticated);
        assert_eq!(
            error.message.as_deref(),
            Some("invalid review access token")
        );
    }

    #[test]
    fn replay_after_upload_returns_existing_job_without_upload_instructions_or_token() {
        let job_id = 42;
        let job = database::ReviewJob {
            job_id,
            tenant_id: "tenant-a".into(),
            idempotency_key: Some("request-1".into()),
            access_token_hash: "a".repeat(64),
            status: DatabaseReviewJobStatus::Completed,
            input_file_path: "reviews/42/source".into(),
            created_at: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
            updated_at: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
            created: false,
        };

        let response = SubmitReviewService::response_for_existing_job(&job, "evaluation-id");

        assert_eq!(response.task_id, job_id.to_string());
        assert_eq!(response.review_id, job_id.to_string());
        assert_eq!(response.evaluation_id, "evaluation-id");
        assert_eq!(response.status, ReviewJobStatus::Completed);
        assert!(response.access_token.is_empty());
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

    #[tokio::test]
    async fn presigns_get_for_the_requested_object() {
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

        let download = presign_download(&client, "uploads-bucket", "reviews/task-1/source")
            .await
            .unwrap();

        assert_eq!(download.method(), "GET");
        assert!(download.uri().starts_with(
            "https://uploads-bucket.s3.eu-west-2.amazonaws.com/reviews/task-1/source?"
        ));
        assert!(download.uri().contains("X-Amz-Expires=900"));
    }
}
