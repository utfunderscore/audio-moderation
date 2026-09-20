use buffa_types::google::protobuf::Timestamp;
use chrono::{DateTime, Duration, Utc};
use common::ReviewAccess;
use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest};
use database::{
    ModerationResult as DatabaseModerationResult,
    PipelineStepErrorDetails as DatabasePipelineStepError,
    PipelineStepStatus as DatabasePipelineStepStatus,
    PipelineTaskDetails as DatabasePipelineTaskDetails, PipelineTaskEventTicketStore,
    PipelineTaskOutcome as DatabasePipelineTaskOutcome,
    PipelineTaskStatus as DatabasePipelineTaskStatus, PipelineTaskStore, ReviewJobStore,
};
use tracing::error;
use uuid::Uuid;

use crate::proto::audio::moderation::v1::{
    AudioModerationService, AudioObjectReference, AudioProcessingTask,
    CreateTaskEventsTicketRequest, CreateTaskEventsTicketResponse, Evaluation,
    GetEvaluationRequest, GetEvaluationResponse, ModerationScores, ModerationTask,
    PipelineStepError, PipelineStepStatus, PipelineTaskEvent, PipelineTaskOutcome,
    PipelineTaskStatus, TranscriptionTask,
};

const AUTHORIZATION_HEADER: &str = "authorization";
const TASK_EVENTS_TICKET_LIFETIME: Duration = Duration::minutes(2);

/// Connect RPC service that exposes persisted evaluations and task-event tickets.
#[derive(Clone)]
pub(crate) struct EvaluationService {
    store: PipelineTaskStore,
    tickets: PipelineTaskEventTicketStore,
    review_access: ReviewAccess,
    reviews: ReviewJobStore,
    tenant_id: String,
}

impl EvaluationService {
    pub(crate) fn new(
        store: PipelineTaskStore,
        tickets: PipelineTaskEventTicketStore,
        review_access: ReviewAccess,
        reviews: ReviewJobStore,
        tenant_id: String,
    ) -> Self {
        Self {
            store,
            tickets,
            review_access,
            reviews,
            tenant_id,
        }
    }

    async fn authorize(
        &self,
        ctx: &RequestContext,
        evaluation_id: &Uuid,
    ) -> Result<(), ConnectError> {
        let value = ctx
            .header(AUTHORIZATION_HEADER)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| ConnectError::unauthenticated("access token is required"))?;
        let (scheme, token) = value
            .trim()
            .split_once(' ')
            .ok_or_else(|| ConnectError::unauthenticated("invalid access token"))?;
        if !scheme.eq_ignore_ascii_case("Bearer") || token.is_empty() {
            return Err(ConnectError::unauthenticated("invalid access token"));
        }
        let Some(review_id) = self.review_access.authenticated_review(
            &self.tenant_id,
            token,
            std::time::SystemTime::now(),
        ) else {
            return Err(ConnectError::unauthenticated("invalid access token"));
        };
        let linked = self.reviews.get_details(review_id, &self.tenant_id).await.map_err(|error| {
            error!(reviewId = review_id, error = ?error, "failed to authorize review evaluation");
            ConnectError::internal("failed to authorize evaluation")
        })?;
        if linked.and_then(|review| review.evaluation_id).as_ref() != Some(evaluation_id) {
            return Err(ConnectError::permission_denied(
                "access token is not scoped to this evaluation",
            ));
        }
        Ok(())
    }
}

impl AudioModerationService for EvaluationService {
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
