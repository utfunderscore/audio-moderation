use std::time::Duration;

use aws_sdk_s3::Client as S3Client;
use aws_sdk_s3::presigning::PresigningConfig;
use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest};
use database::{NewReviewJob, ReviewJobStatus as DatabaseReviewJobStatus, ReviewJobStore};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::proto::audio::review::v1::{
    AudioReviewService, ReviewJobStatus, SubmitReviewRequest, SubmitReviewResponse,
};

const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";
const UPLOAD_URL_TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Clone)]
pub(crate) struct SubmitReviewService {
    store: ReviewJobStore,
    s3_client: S3Client,
    uploads_bucket: String,
    tenant_id: String,
}

impl SubmitReviewService {
    pub(crate) fn new(
        store: ReviewJobStore,
        s3_client: S3Client,
        uploads_bucket: String,
        tenant_id: String,
    ) -> Self {
        Self {
            store,
            s3_client,
            uploads_bucket,
            tenant_id,
        }
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

        let job_id = Uuid::new_v4();
        let input_file_path = format!("reviews/{job_id}/source");
        let job = self
            .store
            .create_or_get(NewReviewJob {
                job_id,
                tenant_id: &self.tenant_id,
                idempotency_key: Some(&idempotency_key),
                input_file_path: &input_file_path,
            })
            .await
            .map_err(|database_error| {
                error!(
                    jobId = %job_id,
                    tenantId = self.tenant_id,
                    outcome = "failed",
                    error = ?database_error,
                    "failed to persist audio submission"
                );
                ConnectError::internal("failed to create review job")
            })?;
        let replayed = job.job_id != job_id;
        if job.status != DatabaseReviewJobStatus::AwaitingUpload {
            info!(
                jobId = %job.job_id,
                tenantId = self.tenant_id,
                status = ?job.status,
                outcome = "idempotent_replay",
                "returned existing audio submission"
            );
            return Response::ok(response_for_existing_job(job.job_id, job.status));
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
                return Response::ok(response_for_existing_job(job.job_id, job.status));
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

        Response::ok(SubmitReviewResponse {
            task_id: job.job_id.to_string(),
            upload_url: upload.uri().to_owned(),
            upload_headers,
            status: ReviewJobStatus::AwaitingUpload.into(),
            ..Default::default()
        })
    }
}

fn response_for_existing_job(
    job_id: Uuid,
    status: DatabaseReviewJobStatus,
) -> SubmitReviewResponse {
    SubmitReviewResponse {
        task_id: job_id.to_string(),
        status: ReviewJobStatus::from(status).into(),
        ..Default::default()
    }
}

impl From<DatabaseReviewJobStatus> for ReviewJobStatus {
    fn from(status: DatabaseReviewJobStatus) -> Self {
        match status {
            DatabaseReviewJobStatus::AwaitingUpload => Self::AwaitingUpload,
            DatabaseReviewJobStatus::Queued => Self::Queued,
            DatabaseReviewJobStatus::StartedPreprocessingAudio => Self::StartedPreprocessingAudio,
            DatabaseReviewJobStatus::FinishedPreprocessingAudio => Self::FinishedPreprocessingAudio,
            DatabaseReviewJobStatus::StartedTranscribing => Self::StartedTranscribing,
            DatabaseReviewJobStatus::FinishedTranscribing => Self::FinishedTranscribing,
            DatabaseReviewJobStatus::StartedEvaluating => Self::StartedEvaluating,
            DatabaseReviewJobStatus::FinishedEvaluating => Self::FinishedEvaluating,
            DatabaseReviewJobStatus::StartedPersistingResult => Self::StartedPersistingResult,
            DatabaseReviewJobStatus::Completed => Self::Completed,
            DatabaseReviewJobStatus::Failed => Self::Failed,
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

    #[test]
    fn replay_after_upload_returns_existing_job_without_upload_instructions() {
        let job_id = Uuid::parse_str("b9de9954-8f85-49e7-82ad-fbe8f2b017a4").unwrap();

        let response = response_for_existing_job(job_id, DatabaseReviewJobStatus::Completed);

        assert_eq!(response.task_id, job_id.to_string());
        assert_eq!(response.status, ReviewJobStatus::Completed);
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
