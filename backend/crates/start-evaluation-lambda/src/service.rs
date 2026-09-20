use std::time::Duration;

use aws_sdk_s3::Client as S3Client;
use aws_sdk_s3::presigning::PresigningConfig;
use common::evaluation_dispatch::EvaluationDispatcher;
use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest};
use database::{
    NewPipelineTask, NewPipelineUpload, PipelineTaskStatus as DatabasePipelineTaskStatus,
    PipelineTaskStore,
};
use tracing::{error, info, warn};

use crate::proto::audio::moderation::v1::{
    AudioModerationService, PipelineTaskStatus, StartEvaluationRequest, StartEvaluationResponse,
};

const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";
const UPLOAD_URL_TTL: Duration = Duration::from_secs(15 * 60);

/// Connect RPC service that creates the durable evaluation and issues its
/// browser-upload URL. The S3 confirmation Lambda owns pipeline dispatch.
#[derive(Clone)]
pub(crate) struct StartEvaluationService {
    store: PipelineTaskStore,
    s3_client: S3Client,
    uploads_bucket: String,
    tenant_id: String,
    dispatcher: EvaluationDispatcher,
}

impl StartEvaluationService {
    pub(crate) fn new(
        store: PipelineTaskStore,
        s3_client: S3Client,
        uploads_bucket: String,
        tenant_id: String,
        dispatcher: EvaluationDispatcher,
    ) -> Self {
        Self {
            store,
            s3_client,
            uploads_bucket,
            tenant_id,
            dispatcher,
        }
    }
}

impl AudioModerationService for StartEvaluationService {
    async fn start_evaluation<'a>(
        &'a self,
        ctx: RequestContext,
        request: ServiceRequest<'_, StartEvaluationRequest>,
    ) -> connectrpc::ServiceResult<
        impl connectrpc::Encodable<StartEvaluationResponse> + Send + use<'a>,
    > {
        let idempotency_key = required_header(&ctx, IDEMPOTENCY_KEY_HEADER)?;
        let caller_reference = optional_string(&request.caller_reference);
        if !request.audio_objects.is_empty() {
            if !request.content_type.trim().is_empty() {
                return Err(ConnectError::invalid_argument(
                    "content_type and audio_objects cannot be used together",
                ));
            }
            let audio_s3_uris =
                validate_audio_objects(request.audio_objects.iter().map(|object| object.s3_uri))?;
            let task = self
                .store
                .create_or_get(NewPipelineTask {
                    tenant_id: &self.tenant_id,
                    idempotency_key: &idempotency_key,
                    caller_reference,
                    audio_s3_uris: &audio_s3_uris,
                })
                .await
                .map_err(|database_error| {
                    error!(tenantId = self.tenant_id, error = ?database_error, "failed to create direct evaluation");
                    ConnectError::internal("failed to create evaluation")
                })?;
            if task.audio_s3_uris != audio_s3_uris
                || task.caller_reference.as_deref() != caller_reference
            {
                return Err(ConnectError::already_exists(
                    "idempotency key was already used for a different evaluation",
                ));
            }
            self.dispatcher.dispatch(task.task_id).await.map_err(|error| {
                error!(evaluationId = task.task_id, error = ?error, "failed to dispatch direct evaluation");
                ConnectError::unavailable("failed to dispatch evaluation")
            })?;
            return Response::ok(StartEvaluationResponse {
                evaluation_id: task.task_id.to_string(),
                status: PipelineTaskStatus::from(task.status).into(),
                ..Default::default()
            });
        }

        let content_type = validate_content_type(&request.content_type)?;
        let task = self
            .store
            .create_upload_or_get(NewPipelineUpload {
                tenant_id: &self.tenant_id,
                idempotency_key: &idempotency_key,
                caller_reference,
                content_type,
                uploads_bucket: &self.uploads_bucket,
            })
            .await
            .map_err(|database_error| {
                error!(tenantId = self.tenant_id, error = ?database_error, "failed to create evaluation upload");
                ConnectError::internal("failed to create evaluation")
            })?;

        if task.caller_reference.as_deref() != caller_reference
            || task.source_upload_content_type.as_deref() != Some(content_type)
        {
            warn!(
                evaluationId = task.task_id,
                tenantId = self.tenant_id,
                "idempotency key was reused with a different evaluation upload request"
            );
            return Err(ConnectError::already_exists(
                "idempotency key was already used for a different evaluation",
            ));
        }

        let (upload_url, upload_headers) = if task.source_upload_confirmed {
            (String::new(), Default::default())
        } else {
            let source_key = evaluation_upload_key(&task.audio_s3_uris, &self.uploads_bucket)?;
            let upload = presign_upload(&self.s3_client, &self.uploads_bucket, source_key, content_type)
                .await
                .map_err(|presign_error| {
                    error!(evaluationId = task.task_id, tenantId = self.tenant_id, error = ?presign_error, "failed to issue evaluation upload URL");
                    presign_error
                })?;
            (
                upload.uri().to_owned(),
                upload
                    .headers()
                    .map(|(name, value)| (name.to_owned(), value.to_owned()))
                    .collect(),
            )
        };

        info!(
            evaluationId = task.task_id,
            tenantId = self.tenant_id,
            status = ?task.status,
            replayed = !task.created,
            "created evaluation upload"
        );
        Response::ok(StartEvaluationResponse {
            evaluation_id: task.task_id.to_string(),
            status: PipelineTaskStatus::from(task.status).into(),
            upload_url,
            upload_headers,
            ..Default::default()
        })
    }
}

fn evaluation_upload_key<'a>(
    audio_s3_uris: &'a [String],
    uploads_bucket: &str,
) -> Result<&'a str, ConnectError> {
    let Some(source_uri) = audio_s3_uris.first() else {
        return Err(ConnectError::internal(
            "evaluation upload has no source object",
        ));
    };
    let prefix = format!("s3://{uploads_bucket}/");
    source_uri.strip_prefix(&prefix).ok_or_else(|| {
        ConnectError::internal("evaluation upload is not in the configured uploads bucket")
    })
}

async fn presign_upload(
    s3_client: &S3Client,
    bucket: &str,
    key: &str,
    content_type: &str,
) -> Result<aws_sdk_s3::presigning::PresignedRequest, ConnectError> {
    let config = PresigningConfig::expires_in(UPLOAD_URL_TTL)
        .map_err(|_| ConnectError::internal("failed to configure upload URL"))?;
    s3_client
        .put_object()
        .bucket(bucket)
        .key(key)
        .content_type(content_type)
        .presigned(config)
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

fn optional_string(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn validate_audio_objects<'a>(
    audio_s3_uris: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<String>, ConnectError> {
    let audio_s3_uris: Vec<_> = audio_s3_uris.into_iter().collect();
    if audio_s3_uris.is_empty() {
        return Err(ConnectError::invalid_argument(
            "audio_objects must contain at least one object",
        ));
    }
    audio_s3_uris
        .into_iter()
        .map(|audio_s3_uri| {
            let uri = audio_s3_uri.trim();
            let Some(location) = uri.strip_prefix("s3://") else {
                return Err(ConnectError::invalid_argument(
                    "audio object references must be S3 URIs",
                ));
            };
            if !location.contains('/') || location.starts_with('/') || location.ends_with('/') {
                return Err(ConnectError::invalid_argument(
                    "audio object references must include a bucket and key",
                ));
            }
            Ok(uri.to_owned())
        })
        .collect()
}

impl From<DatabasePipelineTaskStatus> for PipelineTaskStatus {
    fn from(status: DatabasePipelineTaskStatus) -> Self {
        match status {
            DatabasePipelineTaskStatus::AwaitingUpload => Self::AwaitingUpload,
            DatabasePipelineTaskStatus::Pending => Self::Pending,
            DatabasePipelineTaskStatus::StartedAudioProcessing => Self::StartedAudioProcessing,
            DatabasePipelineTaskStatus::AudioProcessingFinished => Self::AudioProcessingFinished,
            DatabasePipelineTaskStatus::StartedAsr => Self::StartedAsr,
            DatabasePipelineTaskStatus::AsrFinished => Self::AsrFinished,
            DatabasePipelineTaskStatus::StartedModerationProcessing => {
                Self::StartedModerationProcessing
            }
            DatabasePipelineTaskStatus::ModerationProcessingFinished => {
                Self::ModerationProcessingFinished
            }
            DatabasePipelineTaskStatus::Succeeded => Self::Succeeded,
            DatabasePipelineTaskStatus::Failed => Self::Failed,
            DatabasePipelineTaskStatus::TimedOut => Self::TimedOut,
            DatabasePipelineTaskStatus::Cancelled => Self::Cancelled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_content_type() {
        assert_eq!(validate_content_type(" audio/wav ").unwrap(), "audio/wav");
        assert!(validate_content_type("").is_err());
        assert!(validate_content_type("video/mp4").is_err());
    }

    #[test]
    fn extracts_the_configured_upload_key() {
        let inputs = vec!["s3://uploads/evaluations/42/source".to_owned()];
        assert_eq!(
            evaluation_upload_key(&inputs, "uploads").unwrap(),
            "evaluations/42/source"
        );
        assert!(evaluation_upload_key(&inputs, "other").is_err());
    }

    #[test]
    fn validates_direct_audio_object_references() {
        assert_eq!(
            validate_audio_objects([" s3://uploads/audio.wav "]).unwrap(),
            ["s3://uploads/audio.wav"]
        );
        assert!(validate_audio_objects(["https://example.com/audio.wav"]).is_err());
    }
}
