//! Database access and persistence for the workspace.

pub mod pipeline_task_event_store;
pub mod pipeline_task_event_ticket_store;
pub mod pipeline_task_store;
pub mod pipeline_task_websocket_connection_store;
pub mod review_job_store;

pub use pipeline_task_event_store::{
    NewPipelineTaskEvent, PipelineTaskEvent, PipelineTaskEventError, PipelineTaskEventStore,
};
pub use pipeline_task_event_ticket_store::{
    PipelineTaskEventTicket, PipelineTaskEventTicketError, PipelineTaskEventTicketStore,
};

pub use pipeline_task_store::{
    AudioProcessingTaskDetails, CallbackAttempt, DEFAULT_DISPATCH_LEASE, ModerationResult,
    ModerationTaskDetails, NewPipelineTask, PipelineCallbackStep, PipelineStepErrorDetails,
    PipelineStepStatus, PipelineTask, PipelineTaskDetails, PipelineTaskError,
    PipelineTaskEventDetails, PipelineTaskOutcome, PipelineTaskStatus, PipelineTaskStore,
    RecordExecutionResult, TranscriptionTaskDetails,
};

pub use pipeline_task_websocket_connection_store::{
    NewPipelineTaskWebSocketConnection, PipelineTaskWebSocketConnection,
    PipelineTaskWebSocketConnectionError, PipelineTaskWebSocketConnectionStore,
};

pub use review_job_store::{
    DatabaseError, NewReviewJob, ReviewJob, ReviewJobDetails, ReviewJobStatus, ReviewJobStore,
};
