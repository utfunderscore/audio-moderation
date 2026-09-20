//! Idempotent dispatch of a confirmed evaluation upload into Step Functions.

use aws_sdk_sfn::Client as SfnClient;
use database::{PipelineTaskError, PipelineTaskStore};
use serde::Serialize;
use task_event_emitter::TaskEventEmitter;

#[derive(Debug, thiserror::Error)]
pub enum EvaluationDispatchError {
    #[error("failed to access evaluation dispatch state")]
    Store(#[source] PipelineTaskError),
    #[error("failed to serialize evaluation execution input")]
    Serialization(#[source] serde_json::Error),
    #[error("failed to start evaluation execution: {0}")]
    Start(String),
    #[error("failed to emit evaluation lifecycle event")]
    Event(#[source] task_event_emitter::Error),
    #[error("evaluation task {0} does not exist")]
    TaskNotFound(i32),
}

/// Starts a confirmed evaluation exactly once, allowing recovery from a lost
/// Step Functions response through the persisted dispatch lease.
#[derive(Clone)]
pub struct EvaluationDispatcher {
    store: PipelineTaskStore,
    step_functions: SfnClient,
    state_machine_arn: String,
    artifacts_bucket: String,
    events: TaskEventEmitter,
}

impl EvaluationDispatcher {
    pub fn new(
        store: PipelineTaskStore,
        step_functions: SfnClient,
        state_machine_arn: impl Into<String>,
        artifacts_bucket: impl Into<String>,
        events: TaskEventEmitter,
    ) -> Self {
        Self {
            store,
            step_functions,
            state_machine_arn: state_machine_arn.into(),
            artifacts_bucket: artifacts_bucket.into(),
            events,
        }
    }

    /// Claims the task's dispatch lease. A task still awaiting upload, already
    /// dispatched, or owned by another invocation is a harmless no-op.
    pub async fn dispatch(&self, task_id: i32) -> Result<(), EvaluationDispatchError> {
        let task = self
            .store
            .get(task_id)
            .await
            .map_err(EvaluationDispatchError::Store)?
            .ok_or(EvaluationDispatchError::TaskNotFound(task_id))?;
        let Some(attempt) = self
            .store
            .claim_dispatch(task.task_id)
            .await
            .map_err(EvaluationDispatchError::Store)?
        else {
            return Ok(());
        };

        if let Err(error) = self.events.emit(task.task_id, "EVALUATION_ACCEPTED").await {
            self.record_failure(task.task_id).await?;
            return Err(EvaluationDispatchError::Event(error));
        }

        let input = execution_input(task.task_id, &task.audio_s3_uris, &self.artifacts_bucket)
            .map_err(EvaluationDispatchError::Serialization)?;
        let execution_arn = match start_or_recover_execution(
            &self.step_functions,
            &self.state_machine_arn,
            task.task_id,
            attempt,
            input,
        )
        .await
        {
            Ok(execution_arn) => execution_arn,
            Err(error) => {
                self.record_failure(task.task_id).await?;
                return Err(EvaluationDispatchError::Start(error));
            }
        };

        self.store
            .record_execution(task.task_id, &execution_arn)
            .await
            .map_err(EvaluationDispatchError::Store)
    }

    async fn record_failure(&self, task_id: i32) -> Result<(), EvaluationDispatchError> {
        let terminal = self
            .store
            .record_dispatch_failure(task_id)
            .await
            .map_err(EvaluationDispatchError::Store)?;
        if terminal {
            self.events
                .emit(task_id, "FAILED")
                .await
                .map_err(EvaluationDispatchError::Event)?;
        }
        Ok(())
    }
}

async fn start_or_recover_execution(
    step_functions: &SfnClient,
    state_machine_arn: &str,
    task_id: i32,
    dispatch_attempt: i32,
    input: String,
) -> Result<String, String> {
    let execution_name = format!("evaluation-{task_id}");
    let expected_execution_arn = execution_arn(state_machine_arn, &execution_name)?;

    if dispatch_attempt > 1 {
        match step_functions
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

    step_functions
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
        output_s3_uri: format!("s3://{artifacts_bucket}/evaluations/{task_id}.wav"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_input_order_in_execution_payload() {
        let payload = execution_input(42, &["s3://uploads/42/source".into()], "artifacts").unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&payload).unwrap(),
            serde_json::json!({
                "jobId": "42",
                "files": [{ "s3Uri": "s3://uploads/42/source", "sequence": 0 }],
                "outputS3Uri": "s3://artifacts/evaluations/42.wav"
            })
        );
    }
}
