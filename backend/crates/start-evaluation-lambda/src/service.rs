use aws_sdk_sfn::Client as SfnClient;
use buffa_types::google::protobuf::Timestamp;
use chrono::{DateTime, Duration, Utc};
use common::{EvaluationAccess, review_token_hash, review_token_hash_matches};
use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest};
use database::{
    ModerationResult as DatabaseModerationResult, NewPipelineTask,
    PipelineStepErrorDetails as DatabasePipelineStepError,
    PipelineStepStatus as DatabasePipelineStepStatus,
    PipelineTaskDetails as DatabasePipelineTaskDetails, PipelineTaskEventTicketStore,
    PipelineTaskOutcome as DatabasePipelineTaskOutcome,
    PipelineTaskStatus as DatabasePipelineTaskStatus, PipelineTaskStore, ReviewJobStore,
};
use serde::Serialize;
use task_event_emitter::TaskEventEmitter;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::proto::audio::moderation::v1::{
    AudioModerationService, AudioObjectReference, AudioProcessingTask,
    CreateTaskEventsTicketRequest, CreateTaskEventsTicketResponse, Evaluation,
    GetEvaluationRequest, GetEvaluationResponse, ModerationScores, ModerationTask,
    PipelineStepError, PipelineStepStatus, PipelineTaskEvent, PipelineTaskOutcome,
    PipelineTaskStatus, StartEvaluationRequest, StartEvaluationResponse, TranscriptionTask,
};

const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";
const AUTHORIZATION_HEADER: &str = "authorization";
const TASK_EVENTS_TICKET_LIFETIME: Duration = Duration::minutes(2);

/// Connect RPC service that owns evaluation persistence and Step Functions dispatch.
#[derive(Clone)]
pub(crate) struct StartEvaluationService {
    store: PipelineTaskStore,
    tickets: PipelineTaskEventTicketStore,
    reviews: ReviewJobStore,
    access: EvaluationAccess,
    sfn_client: SfnClient,
    state_machine_arn: String,
    artifacts_bucket: String,
    tenant_id: String,
    events: Option<TaskEventEmitter>,
}

impl StartEvaluationService {
    pub(crate) fn new(
        store: PipelineTaskStore,
        tickets: PipelineTaskEventTicketStore,
        reviews: ReviewJobStore,
        access: EvaluationAccess,
        sfn_client: SfnClient,
        state_machine_arn: String,
        artifacts_bucket: String,
        tenant_id: String,
    ) -> Self {
        Self {
            store,
            tickets,
            reviews,
            access,
            sfn_client,
            state_machine_arn,
            artifacts_bucket,
            tenant_id,
            events: None,
        }
    }

    pub(crate) fn with_event_emitter(mut self, events: TaskEventEmitter) -> Self {
        self.events = Some(events);
        self
    }

    async fn authorize(
        &self,
        ctx: &RequestContext,
        evaluation_id: &Uuid,
    ) -> Result<(), ConnectError> {
        let value = ctx
            .header(AUTHORIZATION_HEADER)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| ConnectError::unauthenticated("evaluation access token is required"))?;
        let (scheme, token) = value
            .trim()
            .split_once(' ')
            .ok_or_else(|| ConnectError::unauthenticated("invalid evaluation access token"))?;
        if !scheme.eq_ignore_ascii_case("Bearer") || token.is_empty() {
            return Err(ConnectError::unauthenticated(
                "invalid evaluation access token",
            ));
        }

        // Preserve `eval_v1` direct-evaluation capabilities. Opaque review
        // tokens are resolved through the explicit review_job_id foreign-key.
        if self.access.verify(&evaluation_id.to_string(), token) {
            return Ok(());
        }
        let Some(token_hash) = review_token_hash(token) else {
            return Err(ConnectError::unauthenticated(
                "invalid evaluation access token",
            ));
        };
        let linked = self
            .reviews
            .get_by_evaluation_id(evaluation_id, &self.tenant_id)
            .await
            .map_err(|error| {
                error!(evaluationId = %evaluation_id, error = ?error, "failed to authorize review evaluation");
                ConnectError::internal("failed to authorize evaluation")
            })?;
        if linked
            .is_none_or(|review| !review_token_hash_matches(&review.access_token_hash, &token_hash))
        {
            return Err(ConnectError::unauthenticated(
                "invalid evaluation access token",
            ));
        }
        Ok(())
    }
}

impl AudioModerationService for StartEvaluationService {
    async fn get_evaluation<'a>(
        &'a self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetEvaluationRequest>,
    ) -> connectrpc::ServiceResult<impl connectrpc::Encodable<GetEvaluationResponse> + Send + use<'a>>
    {
        let evaluation_id = parse_evaluation_id(request.evaluation_id)?;
        self.authorize(&ctx, &evaluation_id).await?;
        let details = self
            .store
            .get_details(evaluation_id, &self.tenant_id)
            .await
            .map_err(map_get_error)?;

        Response::ok(GetEvaluationResponse {
            evaluation: evaluation(details).into(),
            ..Default::default()
        })
    }

    async fn create_task_events_ticket<'a>(
        &'a self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateTaskEventsTicketRequest>,
    ) -> connectrpc::ServiceResult<
        impl connectrpc::Encodable<CreateTaskEventsTicketResponse> + Send + use<'a>,
    > {
        let evaluation_id = parse_evaluation_id(request.evaluation_id)?;
        self.authorize(&ctx, &evaluation_id).await?;
        let details = self
            .store
            .get_details(evaluation_id, &self.tenant_id)
            .await
            .map_err(map_get_error)?;
        let ticket = format!("wst_v1.{}", Uuid::new_v4().simple());
        let persisted = self
            .tickets
            .create(details.task_id, &ticket, TASK_EVENTS_TICKET_LIFETIME)
            .await
            .map_err(|error| {
                error!(evaluationId = %evaluation_id, error = ?error, "failed to create task-events ticket");
                ConnectError::internal("failed to create task-events ticket")
            })?;

        Response::ok(CreateTaskEventsTicketResponse {
            ticket,
            expires_at: timestamp(persisted.expires_at).into(),
            ..Default::default()
        })
    }

    /// Creates or replays an evaluation and ensures that one invocation owns
    /// the attempt to dispatch it.
    async fn start_evaluation<'a>(
        &'a self,
        ctx: RequestContext,
        request: ServiceRequest<'_, StartEvaluationRequest>,
    ) -> connectrpc::ServiceResult<
        impl connectrpc::Encodable<StartEvaluationResponse> + Send + use<'a>,
    > {
        // Step 1: Validate caller identity for idempotency and normalize the
        // ordered object references before writing anything to the database.
        let idempotency_key = required_header(&ctx, IDEMPOTENCY_KEY_HEADER)?;
        Uuid::parse_str(&idempotency_key)
            .map_err(|_| ConnectError::invalid_argument("idempotency-key must be a UUID"))?;
        let audio_s3_uris =
            validate_audio_objects(request.audio_objects.iter().map(|object| object.s3_uri))?;
        let caller_reference = optional_string(request.caller_reference);

        // Step 2: Create the pipeline task and its inputs in one transaction.
        // If this tenant has already used the idempotency key, the store returns
        // that existing task and its originally persisted inputs instead.
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

        // Step 3: Confirm that an idempotent replay is the same logical request.
        // The key identifies the complete payload, so changed inputs or caller
        // context are rejected rather than silently returning unrelated work.
        if task.audio_s3_uris != audio_s3_uris
            || task.caller_reference.as_deref() != caller_reference
        {
            warn!(
                evaluationId = %task.evaluation_id,
                taskId = task.task_id,
                tenantId = self.tenant_id,
                "idempotency key was reused with a different request"
            );
            return Err(ConnectError::already_exists(
                "idempotency key was already used for a different evaluation",
            ));
        }

        if let Some(events) = &self.events {
            events.emit(task.task_id, "EVALUATION_ACCEPTED").await.map_err(|error| {
                error!(evaluationId = %task.evaluation_id, taskId = task.task_id, error = ?error, "failed to emit evaluation event");
                ConnectError::internal("failed to record evaluation event")
            })?;
        }

        // Step 4: Try to claim responsibility for dispatching an undispatched
        // task. The database lease allows only one concurrent invocation to
        // receive an attempt number. A task with an execution ARN, an active
        // lease, or no retries remaining returns None and is not started here.
        let dispatch_attempt = if task.execution_arn.is_none() {
            self.store
                .claim_dispatch(task.task_id)
                .await
                .map_err(|database_error| {
                    error!(
                        evaluationId = %task.evaluation_id,
                        taskId = task.task_id,
                        error = ?database_error,
                        "failed to claim pipeline dispatch"
                    );
                    ConnectError::internal("failed to dispatch evaluation")
                })?
        } else {
            None
        };

        if let Some(dispatch_attempt) = dispatch_attempt {
            // Step 5: Build the exact JSON contract consumed by the first state
            // machine task, preserving the caller's audio-object order.
            let input = execution_input(task.task_id, &task.audio_s3_uris, &self.artifacts_bucket)
                .map_err(|serialization_error| {
                    error!(
                        evaluationId = %task.evaluation_id,
                        taskId = task.task_id,
                        error = ?serialization_error,
                        "failed to serialize pipeline execution input"
                    );
                    ConnectError::internal("failed to dispatch evaluation")
                })?;
            // Step 6: Start the Standard workflow. On retries, first check for
            // an execution that AWS may have started even though an earlier
            // StartExecution response was lost or timed out.
            let dispatch_result = start_or_recover_execution(
                &self.sfn_client,
                &self.state_machine_arn,
                &task.evaluation_id,
                dispatch_attempt,
                input,
            )
            .await;
            let execution_arn = match dispatch_result {
                Ok(execution_arn) => execution_arn,
                Err(dispatch_error) => {
                    // Step 7a: A confirmed dispatch failure releases the lease
                    // and persists the error. The same idempotency key can retry
                    // until the store marks the third failure as terminal.
                    error!(
                        evaluationId = %task.evaluation_id,
                        taskId = task.task_id,
                        error = ?dispatch_error,
                        "failed to start pipeline execution"
                    );
                    let terminal_failure = self
                        .store
                        .record_dispatch_failure(task.task_id)
                        .await
                        .map_err(|database_error| {
                        error!(
                            evaluationId = %task.evaluation_id,
                            taskId = task.task_id,
                            error = ?database_error,
                            "failed to record pipeline dispatch failure"
                        );
                        ConnectError::internal("failed to record evaluation dispatch failure")
                    })?;
                    if terminal_failure {
                        if let Some(events) = &self.events {
                            events.emit(task.task_id, "FAILED").await.map_err(|error| {
                                error!(evaluationId = %task.evaluation_id, taskId = task.task_id, error = ?error, "failed to emit evaluation failure");
                                ConnectError::internal("failed to record evaluation event")
                            })?;
                        }
                    }
                    return Err(ConnectError::unavailable("failed to dispatch evaluation"));
                }
            };

            // Step 7b: A successful or recovered dispatch stores the execution
            // ARN and releases the lease. Future replays will skip dispatch.
            let _recorded = self
                .store
                .record_execution(task.task_id, &execution_arn)
                .await
                .map_err(|database_error| {
                    error!(
                        evaluationId = %task.evaluation_id,
                        taskId = task.task_id,
                        executionArn = execution_arn,
                        error = ?database_error,
                        "failed to record pipeline execution"
                    );
                    ConnectError::internal("failed to record evaluation dispatch")
                })?;
        }

        // Step 8: Return the durable evaluation identity. This also covers an
        // idempotent replay and a concurrent request whose dispatch is already
        // being handled by the invocation that acquired the lease.
        info!(
            evaluationId = %task.evaluation_id,
            taskId = task.task_id,
            tenantId = self.tenant_id,
            callerReference = task.caller_reference,
            replayed = !task.created,
            "accepted evaluation"
        );

        Response::ok(StartEvaluationResponse {
            access_token: self.access.token(&task.evaluation_id),
            evaluation_id: task.evaluation_id,
            status: PipelineTaskStatus::from(task.status).into(),
            ..Default::default()
        })
    }
}

async fn start_or_recover_execution(
    sfn_client: &SfnClient,
    state_machine_arn: &str,
    evaluation_id: &str,
    dispatch_attempt: i32,
    input: String,
) -> Result<String, String> {
    let execution_name = execution_name(evaluation_id)?;
    let expected_execution_arn = execution_arn(state_machine_arn, &execution_name)?;

    // A previous StartExecution request may have reached AWS even when the
    // caller observed a timeout. Standard workflows have deterministic ARNs,
    // so retries first recover an execution that already exists.
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

fn execution_name(evaluation_id: &str) -> Result<String, String> {
    let evaluation_id =
        Uuid::parse_str(evaluation_id).map_err(|_| "evaluation ID must be a UUID".to_owned())?;
    Ok(format!("evaluation-{evaluation_id}"))
}

fn execution_arn(state_machine_arn: &str, execution_name: &str) -> Result<String, String> {
    let parts: Vec<_> = state_machine_arn.split(':').collect();
    // Qualified state-machine ARNs include extra segments and cannot be mapped
    // to an execution ARN with this deterministic transformation.
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
/// Input contract consumed by the first task in the moderation state machine.
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
    // Sequence is derived from repeated-field order so callers do not need to
    // coordinate a second ordering mechanism.
    serde_json::to_string(&PipelineExecutionInput {
        job_id: task_id.to_string(),
        files: audio_s3_uris
            .iter()
            .enumerate()
            .map(|(sequence, s3_uri)| PipelineAudioSource { s3_uri, sequence })
            .collect(),
        output_s3_uri: format!("s3://{artifacts_bucket}/evaluations/{task_id}.wav"),
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

    // The ingress accepts references only; callers must upload objects before
    // creating an evaluation.
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
    // Proto3 strings have no presence by default, so an empty value represents
    // an omitted caller reference.
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn parse_evaluation_id(value: &str) -> Result<Uuid, ConnectError> {
    Uuid::parse_str(value.trim())
        .map_err(|_| ConnectError::invalid_argument("evaluation_id must be a UUID"))
}

fn map_get_error(error: database::PipelineTaskError) -> ConnectError {
    match error {
        database::PipelineTaskError::NotFound => ConnectError::not_found("evaluation not found"),
        error => {
            error!(error = ?error, "failed to retrieve evaluation");
            ConnectError::internal("failed to retrieve evaluation")
        }
    }
}

fn evaluation(details: DatabasePipelineTaskDetails) -> Evaluation {
    Evaluation {
        evaluation_id: details.evaluation_id.to_string(),
        caller_reference: details.caller_reference,
        status: PipelineTaskStatus::from(details.status).into(),
        outcome: details
            .outcome
            .map(PipelineTaskOutcome::from)
            .map(Into::into),
        audio_objects: details
            .audio_s3_uris
            .into_iter()
            .map(|s3_uri| AudioObjectReference {
                s3_uri,
                ..Default::default()
            })
            .collect(),
        audio_processing: AudioProcessingTask {
            status: PipelineStepStatus::from(details.audio_processing.status).into(),
            stitched_audio_s3_uri: details.audio_processing.stitched_audio_s3_uri,
            error: pipeline_error(details.audio_processing.error).into(),
            started_at: details.audio_processing.started_at.map(timestamp).into(),
            completed_at: details.audio_processing.completed_at.map(timestamp).into(),
            updated_at: timestamp(details.audio_processing.updated_at).into(),
            ..Default::default()
        }
        .into(),
        transcription: TranscriptionTask {
            status: PipelineStepStatus::from(details.transcription.status).into(),
            external_task_id: details.transcription.external_task_id,
            transcript: details.transcription.transcript,
            error: pipeline_error(details.transcription.error).into(),
            started_at: details.transcription.started_at.map(timestamp).into(),
            completed_at: details.transcription.completed_at.map(timestamp).into(),
            updated_at: timestamp(details.transcription.updated_at).into(),
            ..Default::default()
        }
        .into(),
        moderation: ModerationTask {
            status: PipelineStepStatus::from(details.moderation.status).into(),
            external_task_id: details.moderation.external_task_id,
            scores: details.moderation.scores.map(moderation_scores).into(),
            error: pipeline_error(details.moderation.error).into(),
            started_at: details.moderation.started_at.map(timestamp).into(),
            completed_at: details.moderation.completed_at.map(timestamp).into(),
            updated_at: timestamp(details.moderation.updated_at).into(),
            ..Default::default()
        }
        .into(),
        events: details
            .events
            .into_iter()
            .map(|event| PipelineTaskEvent {
                event_id: event.event_id,
                event_name: event.event_name,
                created_at: timestamp(event.created_at).into(),
                ..Default::default()
            })
            .collect(),
        created_at: timestamp(details.created_at).into(),
        updated_at: timestamp(details.updated_at).into(),
        completed_at: details.completed_at.map(timestamp).into(),
        ..Default::default()
    }
}

fn pipeline_error(error: DatabasePipelineStepError) -> Option<PipelineStepError> {
    if error.code.is_none() && error.message.is_none() {
        None
    } else {
        Some(PipelineStepError {
            code: error.code,
            message: error.message,
            ..Default::default()
        })
    }
}

fn moderation_scores(scores: DatabaseModerationResult) -> ModerationScores {
    ModerationScores {
        sexual: scores.sexual,
        hate_or_discrimination: scores.hate_or_discrimination,
        harassment_or_abuse: scores.harassment_or_abuse,
        violence_or_threats: scores.violence_or_threats,
        asking_for_pii: scores.asking_for_pii,
        ..Default::default()
    }
}

fn timestamp(value: DateTime<Utc>) -> Timestamp {
    Timestamp {
        seconds: value.timestamp(),
        nanos: value.timestamp_subsec_nanos() as i32,
        ..Default::default()
    }
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

impl From<DatabasePipelineTaskOutcome> for PipelineTaskOutcome {
    fn from(outcome: DatabasePipelineTaskOutcome) -> Self {
        match outcome {
            DatabasePipelineTaskOutcome::Succeeded => Self::Succeeded,
            DatabasePipelineTaskOutcome::Failed => Self::Failed,
            DatabasePipelineTaskOutcome::TimedOut => Self::TimedOut,
            DatabasePipelineTaskOutcome::Cancelled => Self::Cancelled,
        }
    }
}

impl From<DatabasePipelineStepStatus> for PipelineStepStatus {
    fn from(status: DatabasePipelineStepStatus) -> Self {
        match status {
            DatabasePipelineStepStatus::Pending => Self::Pending,
            DatabasePipelineStepStatus::Processing => Self::Processing,
            DatabasePipelineStepStatus::Completed => Self::Completed,
            DatabasePipelineStepStatus::Failed => Self::Failed,
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
        assert_eq!(input["outputS3Uri"], "s3://artifacts/evaluations/42.wav");
    }

    #[test]
    fn derives_standard_execution_arn() {
        assert_eq!(
            execution_arn(
                "arn:aws:states:eu-west-2:123456789012:stateMachine:moderation",
                "evaluation-4e7c6c8a-4f1f-4b4c-a5a4-45d1ccffc5b2"
            )
            .unwrap(),
            "arn:aws:states:eu-west-2:123456789012:execution:moderation:evaluation-4e7c6c8a-4f1f-4b4c-a5a4-45d1ccffc5b2"
        );
        assert!(
            execution_arn(
                "arn:aws:states:eu-west-2:123456789012:stateMachine:moderation:PROD",
                "evaluation-4e7c6c8a-4f1f-4b4c-a5a4-45d1ccffc5b2"
            )
            .is_err()
        );
    }

    #[test]
    fn derives_execution_name_from_evaluation_uuid() {
        assert_eq!(
            execution_name("4E7C6C8A-4F1F-4B4C-A5A4-45D1CCFFC5B2").unwrap(),
            "evaluation-4e7c6c8a-4f1f-4b4c-a5a4-45d1ccffc5b2"
        );
        assert!(execution_name("42").is_err());
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
