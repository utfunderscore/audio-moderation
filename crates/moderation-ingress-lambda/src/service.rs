use aws_sdk_sfn::Client as SfnClient;
use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest};
use database::{
    NewPipelineTask, PipelineTaskStatus as DatabasePipelineTaskStatus, PipelineTaskStore,
};
use serde::Serialize;
use tracing::{error, info, warn};

use crate::proto::audio::moderation::v1::{
    AudioModerationService, PipelineTaskStatus, StartEvaluationRequest, StartEvaluationResponse,
};

const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";

#[derive(Clone)]
pub(crate) struct ModerationIngressService {
    store: PipelineTaskStore,
    sfn_client: SfnClient,
    state_machine_arn: String,
    artifacts_bucket: String,
    tenant_id: String,
}

impl ModerationIngressService {
    pub(crate) fn new(
        store: PipelineTaskStore,
        sfn_client: SfnClient,
        state_machine_arn: String,
        artifacts_bucket: String,
        tenant_id: String,
    ) -> Self {
        Self {
            store,
            sfn_client,
            state_machine_arn,
            artifacts_bucket,
            tenant_id,
        }
    }
}

impl AudioModerationService for ModerationIngressService {
    async fn start_evaluation<'a>(
        &'a self,
        ctx: RequestContext,
        request: ServiceRequest<'_, StartEvaluationRequest>,
    ) -> connectrpc::ServiceResult<
        impl connectrpc::Encodable<StartEvaluationResponse> + Send + use<'a>,
    > {
        let idempotency_key = required_header(&ctx, IDEMPOTENCY_KEY_HEADER)?;
        let audio_s3_uris =
            validate_audio_objects(request.audio_objects.iter().map(|object| object.s3_uri))?;
        let caller_reference = optional_string(request.caller_reference);

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
                error!(
                    tenantId = self.tenant_id,
                    error = ?database_error,
                    "failed to persist evaluation"
                );
                ConnectError::internal("failed to create evaluation")
            })?;

        if task.audio_s3_uris != audio_s3_uris
            || task.caller_reference.as_deref() != caller_reference
        {
            warn!(
                evaluationId = task.task_id,
                tenantId = self.tenant_id,
                "idempotency key was reused with a different request"
            );
            return Err(ConnectError::already_exists(
                "idempotency key was already used for a different evaluation",
            ));
        }

        let dispatch_attempt = if task.execution_arn.is_none() {
            self.store
                .claim_dispatch(task.task_id)
                .await
                .map_err(|database_error| {
                    error!(
                        evaluationId = task.task_id,
                        error = ?database_error,
                        "failed to claim pipeline dispatch"
                    );
                    ConnectError::internal("failed to dispatch evaluation")
                })?
        } else {
            None
        };

        if let Some(dispatch_attempt) = dispatch_attempt {
            let input = execution_input(task.task_id, &task.audio_s3_uris, &self.artifacts_bucket)
                .map_err(|serialization_error| {
                    error!(
                        evaluationId = task.task_id,
                        error = ?serialization_error,
                        "failed to serialize pipeline execution input"
                    );
                    ConnectError::internal("failed to dispatch evaluation")
                })?;
            let dispatch_result = start_or_recover_execution(
                &self.sfn_client,
                &self.state_machine_arn,
                task.task_id,
                dispatch_attempt,
                input,
            )
            .await;
            let execution_arn = match dispatch_result {
                Ok(execution_arn) => execution_arn,
                Err(dispatch_error) => {
                    error!(
                        evaluationId = task.task_id,
                        error = ?dispatch_error,
                        "failed to start pipeline execution"
                    );
                    self.store
                        .record_dispatch_failure(task.task_id, &dispatch_error)
                        .await
                        .map_err(|database_error| {
                            error!(
                                evaluationId = task.task_id,
                                error = ?database_error,
                                "failed to record pipeline dispatch failure"
                            );
                            ConnectError::internal("failed to record evaluation dispatch failure")
                        })?;
                    return Err(ConnectError::unavailable("failed to dispatch evaluation"));
                }
            };

            self.store
                .record_execution(task.task_id, &execution_arn)
                .await
                .map_err(|database_error| {
                    error!(
                        evaluationId = task.task_id,
                        executionArn = execution_arn,
                        error = ?database_error,
                        "failed to record pipeline execution"
                    );
                    ConnectError::internal("failed to record evaluation dispatch")
                })?;
        }

        info!(
            evaluationId = task.task_id,
            tenantId = self.tenant_id,
            callerReference = task.caller_reference,
            replayed = !task.created,
            "accepted evaluation"
        );

        Response::ok(StartEvaluationResponse {
            evaluation_id: task.task_id.to_string(),
            status: PipelineTaskStatus::from(task.status).into(),
            ..Default::default()
        })
    }
}

async fn start_or_recover_execution(
    sfn_client: &SfnClient,
    state_machine_arn: &str,
    task_id: i32,
    dispatch_attempt: i32,
    input: String,
) -> Result<String, String> {
    let execution_name = format!("evaluation-{task_id}");
    let expected_execution_arn = execution_arn(state_machine_arn, &execution_name)?;

    if dispatch_attempt > 1 {
        match sfn_client
            .describe_execution()
            .execution_arn(&expected_execution_arn)
            .send()
            .await
        {
            Ok(_) => return Ok(expected_execution_arn),
            Err(error)
                if error
                    .as_service_error()
                    .is_some_and(|service_error| service_error.is_execution_does_not_exist()) => {}
            Err(error) => return Err(error.to_string()),
        }
    }

    sfn_client
        .start_execution()
        .state_machine_arn(state_machine_arn)
        .name(execution_name)
        .input(input)
        .send()
        .await
        .map(|output| output.execution_arn().to_owned())
        .map_err(|error| error.to_string())
}

fn execution_arn(state_machine_arn: &str, execution_name: &str) -> Result<String, String> {
    let parts: Vec<_> = state_machine_arn.split(':').collect();
    if parts.len() != 7 || parts[2] != "states" || parts[5] != "stateMachine" {
        return Err("STATE_MACHINE_ARN must be an unqualified Step Functions ARN".to_owned());
    }

    Ok(format!(
        "{}:{}:{}:{}:{}:execution:{}:{execution_name}",
        parts[0], parts[1], parts[2], parts[3], parts[4], parts[6]
    ))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PipelineExecutionInput<'a> {
    job_id: String,
    files: Vec<PipelineAudioSource<'a>>,
    output_s3_uri: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PipelineAudioSource<'a> {
    s3_uri: &'a str,
    sequence: usize,
}

fn execution_input(
    task_id: i32,
    audio_s3_uris: &[String],
    artifacts_bucket: &str,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(&PipelineExecutionInput {
        job_id: task_id.to_string(),
        files: audio_s3_uris
            .iter()
            .enumerate()
            .map(|(sequence, s3_uri)| PipelineAudioSource { s3_uri, sequence })
            .collect(),
        output_s3_uri: format!(
            "s3://{artifacts_bucket}/evaluations/{task_id}/processed/stitched.wav"
        ),
    })
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

impl From<DatabasePipelineTaskStatus> for PipelineTaskStatus {
    fn from(status: DatabasePipelineTaskStatus) -> Self {
        match status {
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
    use crate::proto::audio::moderation::v1::AudioObjectReference;

    #[test]
    fn preserves_audio_object_order_in_execution_input() {
        let input = execution_input(
            42,
            &[
                "s3://uploads/second.wav".into(),
                "s3://uploads/first.wav".into(),
            ],
            "artifacts",
        )
        .unwrap();
        let input: serde_json::Value = serde_json::from_str(&input).unwrap();

        assert_eq!(input["jobId"], "42");
        assert_eq!(input["files"][0]["sequence"], 0);
        assert_eq!(input["files"][0]["s3Uri"], "s3://uploads/second.wav");
        assert_eq!(input["files"][1]["sequence"], 1);
        assert_eq!(
            input["outputS3Uri"],
            "s3://artifacts/evaluations/42/processed/stitched.wav"
        );
    }

    #[test]
    fn derives_standard_execution_arn() {
        assert_eq!(
            execution_arn(
                "arn:aws:states:eu-west-2:123456789012:stateMachine:moderation",
                "evaluation-42"
            )
            .unwrap(),
            "arn:aws:states:eu-west-2:123456789012:execution:moderation:evaluation-42"
        );
        assert!(
            execution_arn(
                "arn:aws:states:eu-west-2:123456789012:stateMachine:moderation:PROD",
                "evaluation-42"
            )
            .is_err()
        );
    }

    #[test]
    fn validates_audio_objects() {
        let request = StartEvaluationRequest {
            audio_objects: vec![AudioObjectReference {
                s3_uri: " s3://uploads/audio.wav ".into(),
                ..Default::default()
            }],
            ..Default::default()
        };

        assert_eq!(
            validate_audio_objects(
                request
                    .audio_objects
                    .iter()
                    .map(|object| object.s3_uri.as_str())
            )
            .unwrap(),
            vec!["s3://uploads/audio.wav"]
        );
    }

    #[test]
    fn rejects_missing_and_invalid_audio_objects() {
        let empty = StartEvaluationRequest::default();
        assert!(
            validate_audio_objects(
                empty
                    .audio_objects
                    .iter()
                    .map(|object| object.s3_uri.as_str())
            )
            .is_err()
        );

        let invalid = StartEvaluationRequest {
            audio_objects: vec![AudioObjectReference {
                s3_uri: "https://example.com/audio.wav".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(
            validate_audio_objects(
                invalid
                    .audio_objects
                    .iter()
                    .map(|object| object.s3_uri.as_str())
            )
            .is_err()
        );
    }
}
