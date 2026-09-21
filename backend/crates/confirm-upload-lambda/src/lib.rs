use std::borrow::Cow;

use aws_lambda_events::event::s3::S3Event;
use aws_sdk_s3::Client as S3Client;
use aws_sdk_sfn::Client as SfnClient;
use database::{PipelineTaskStatus, PipelineTaskStore, ReviewJobStatus, ReviewJobStore};
use lambda_runtime::{Error, LambdaEvent};
use serde::Serialize;
use task_event_emitter::TaskEventEmitter;
use tracing::info;

#[derive(Clone)]
pub struct ConfirmUploadHandler {
    store: ReviewJobStore,
    pipeline_store: PipelineTaskStore,
    s3_client: S3Client,
    sfn_client: SfnClient,
    uploads_bucket: String,
    artifacts_bucket: String,
    state_machine_arn: String,
    tenant_id: String,
    events: TaskEventEmitter,
}

impl ConfirmUploadHandler {
    pub fn new(
        store: ReviewJobStore,
        pipeline_store: PipelineTaskStore,
        s3_client: S3Client,
        sfn_client: SfnClient,
        uploads_bucket: String,
        artifacts_bucket: String,
        state_machine_arn: String,
        tenant_id: String,
        events: TaskEventEmitter,
    ) -> Self {
        Self {
            store,
            pipeline_store,
            s3_client,
            sfn_client,
            uploads_bucket,
            artifacts_bucket,
            state_machine_arn,
            tenant_id,
            events,
        }
    }

    pub async fn handle(&self, event: LambdaEvent<S3Event>) -> Result<(), Error> {
        let request_id = event.context.request_id;
        let objects = validate_correct_bucket(&event.payload, &self.uploads_bucket)?;

        for object in objects {
            self.s3_client
                .head_object()
                .bucket(&object.bucket)
                .key(&object.key)
                .send()
                .await?;

            let job = self
                .store
                .get_by_input_file_path(&object.key, &self.tenant_id)
                .await?;
            let Some(job) = job else {
                return Err(ConfirmUploadError::UnknownObject(object.key).into());
            };

            let audio_s3_uris = vec![format!("s3://{}/{}", object.bucket, object.key)];
            let idempotency_key = format!("review-upload:{}", job.job_id);
            let caller_reference = format!("review-job:{}", job.job_id);
            let task = self
                .pipeline_store
                .get_by_review_job(job.job_id, &self.tenant_id)
                .await?
                .ok_or(ConfirmUploadError::PipelineTaskMissing(job.job_id))?;

            if task.review_job_id != Some(job.job_id)
                || task.audio_s3_uris != audio_s3_uris
                || task.idempotency_key != idempotency_key
                || task.caller_reference.as_deref() != Some(caller_reference.as_str())
            {
                return Err(ConfirmUploadError::PipelineTaskConflict(job.job_id).into());
            }

            self.store
                .mark_upload_complete(&object.key, &self.tenant_id)
                .await?;
            self.events.emit(task.task_id, "UPLOAD_RECEIVED").await?;

            if matches!(
                task.status,
                PipelineTaskStatus::Failed
                    | PipelineTaskStatus::TimedOut
                    | PipelineTaskStatus::Cancelled
            ) {
                self.store
                    .update_status(job.job_id, &self.tenant_id, ReviewJobStatus::Error)
                    .await?;
                continue;
            }

            if task.status == PipelineTaskStatus::Succeeded {
                self.store
                    .update_status(job.job_id, &self.tenant_id, ReviewJobStatus::Completed)
                    .await?;
                continue;
            }

            if task.execution_arn.is_none() {
                let Some(dispatch_attempt) =
                    self.pipeline_store.claim_dispatch(task.task_id).await?
                else {
                    // An overlapping notification may own the dispatch lease. Returning an
                    // error preserves S3's retry until that invocation records its execution.
                    return Err(ConfirmUploadError::DispatchNotReady(job.job_id).into());
                };
                let input =
                    execution_input(task.task_id, &task.audio_s3_uris, &self.artifacts_bucket)?;
                let execution = start_or_recover_execution(
                    &self.sfn_client,
                    &self.state_machine_arn,
                    &task.evaluation_id,
                    dispatch_attempt,
                    input,
                )
                .await;
                let execution_arn = match execution {
                    Ok(execution_arn) => execution_arn,
                    Err(error) => {
                        let terminal = self
                            .pipeline_store
                            .record_dispatch_failure(task.task_id)
                            .await?;
                        if terminal {
                            self.store
                                .update_status(job.job_id, &self.tenant_id, ReviewJobStatus::Error)
                                .await?;
                        }
                        return Err(ConfirmUploadError::DispatchFailed(error).into());
                    }
                };
                let _recorded = self
                    .pipeline_store
                    .record_execution(task.task_id, &execution_arn)
                    .await?;
            }

            self.store
                .update_status(job.job_id, &self.tenant_id, ReviewJobStatus::Processing)
                .await?;

            info!(
                requestId = request_id,
                jobId = %job.job_id,
                tenantId = job.tenant_id,
                bucket = object.bucket,
                objectKey = object.key,
                evaluationId = task.evaluation_id,
                pipelineTaskId = task.task_id,
                status = ?ReviewJobStatus::Processing,
                outcome = "evaluation_started",
                "started evaluation for source audio upload"
            );
        }

        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
struct UploadedObject {
    bucket: String,
    key: String,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
enum ConfirmUploadError {
    #[error("S3 event did not contain any records")]
    EmptyEvent,
    #[error("S3 event record did not contain a bucket name")]
    MissingBucket,
    #[error("S3 event record did not contain an object key")]
    MissingKey,
    #[error("received upload event from unexpected bucket {0}")]
    UnexpectedBucket(String),
    #[error("S3 object key is not valid UTF-8 after decoding")]
    InvalidKeyEncoding,
    #[error("no review job exists for uploaded object {0}")]
    UnknownObject(String),
    #[error("review job {0} is associated with conflicting pipeline inputs")]
    PipelineTaskConflict(i32),
    #[error("review job {0} is missing its pre-created pipeline task")]
    PipelineTaskMissing(i32),
    #[error("evaluation dispatch for review job {0} is not ready")]
    DispatchNotReady(i32),
    #[error("failed to dispatch evaluation: {0}")]
    DispatchFailed(String),
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

async fn start_or_recover_execution(
    client: &SfnClient,
    state_machine_arn: &str,
    evaluation_id: &str,
    dispatch_attempt: i32,
    input: String,
) -> Result<String, String> {
    let execution_name = format!("evaluation-{evaluation_id}");
    let expected_arn = execution_arn(state_machine_arn, &execution_name)?;

    if dispatch_attempt > 1 {
        match client
            .describe_execution()
            .execution_arn(&expected_arn)
            .send()
            .await
        {
            Ok(_) => return Ok(expected_arn),
            Err(error)
                if error
                    .as_service_error()
                    .is_some_and(|service_error| service_error.is_execution_does_not_exist()) => {}
            Err(error) => return Err(error.to_string()),
        }
    }

    client
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

fn validate_correct_bucket(
    event: &S3Event,
    expected_bucket: &str,
) -> Result<Vec<UploadedObject>, ConfirmUploadError> {
    if event.records.is_empty() {
        return Err(ConfirmUploadError::EmptyEvent);
    }

    event
        .records
        .iter()
        .map(|record| {
            let bucket = record
                .s3
                .bucket
                .name
                .clone()
                .ok_or(ConfirmUploadError::MissingBucket)?;
            if bucket != expected_bucket {
                return Err(ConfirmUploadError::UnexpectedBucket(bucket));
            }

            let encoded_key = record
                .s3
                .object
                .key
                .as_deref()
                .ok_or(ConfirmUploadError::MissingKey)?;
            let key = decode_s3_key(encoded_key)?;

            Ok(UploadedObject { bucket, key })
        })
        .collect()
}

fn decode_s3_key(encoded_key: &str) -> Result<String, ConfirmUploadError> {
    let encoded_key = if encoded_key.contains('+') {
        Cow::Owned(encoded_key.replace('+', " "))
    } else {
        Cow::Borrowed(encoded_key)
    };

    urlencoding::decode(&encoded_key)
        .map(Cow::into_owned)
        .map_err(|_| ConfirmUploadError::InvalidKeyEncoding)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s3_event(bucket: &str, key: &str) -> S3Event {
        serde_json::from_value(serde_json::json!({
            "Records": [{
                "eventVersion": "2.1",
                "eventSource": "aws:s3",
                "awsRegion": "eu-west-2",
                "eventTime": "2026-09-04T10:00:00Z",
                "eventName": "ObjectCreated:Put",
                "userIdentity": { "principalId": "test" },
                "requestParameters": { "sourceIPAddress": "127.0.0.1" },
                "responseElements": {
                    "x-amz-request-id": "request-id",
                    "x-amz-id-2": "host-id"
                },
                "s3": {
                    "s3SchemaVersion": "1.0",
                    "configurationId": "confirm-upload",
                    "bucket": {
                        "name": bucket,
                        "ownerIdentity": { "principalId": "test" },
                        "arn": format!("arn:aws:s3:::{bucket}")
                    },
                    "object": {
                        "key": key,
                        "size": 44,
                        "eTag": "etag",
                        "sequencer": "001"
                    }
                }
            }]
        }))
        .unwrap()
    }

    #[test]
    fn extracts_and_decodes_uploaded_object() {
        let event = s3_event("uploads", "reviews%2Ftask+one%2Fsource");

        assert_eq!(
            validate_correct_bucket(&event, "uploads").unwrap(),
            vec![UploadedObject {
                bucket: "uploads".to_owned(),
                key: "reviews/task one/source".to_owned(),
            }]
        );
    }

    #[test]
    fn rejects_an_unexpected_bucket() {
        let error = validate_correct_bucket(&s3_event("other", "reviews/task/source"), "uploads")
            .unwrap_err();

        assert_eq!(
            error,
            ConfirmUploadError::UnexpectedBucket("other".to_owned())
        );
    }

    #[test]
    fn rejects_an_empty_event() {
        let event: S3Event = serde_json::from_value(serde_json::json!({ "Records": [] })).unwrap();

        assert_eq!(
            validate_correct_bucket(&event, "uploads").unwrap_err(),
            ConfirmUploadError::EmptyEvent
        );
    }

    #[test]
    fn builds_pipeline_input_for_uploaded_audio() {
        let input = execution_input(
            42,
            &["s3://uploads/reviews/7/source".to_owned()],
            "artifacts",
        )
        .unwrap();

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&input).unwrap(),
            serde_json::json!({
                "jobId": "42",
                "files": [{
                    "s3Uri": "s3://uploads/reviews/7/source",
                    "sequence": 0
                }],
                "outputS3Uri": "s3://artifacts/evaluations/42.wav"
            })
        );
    }

    #[test]
    fn derives_standard_workflow_execution_arn() {
        assert_eq!(
            execution_arn(
                "arn:aws:states:eu-west-2:123456789012:stateMachine:audio",
                "evaluation-id"
            )
            .unwrap(),
            "arn:aws:states:eu-west-2:123456789012:execution:audio:evaluation-id"
        );
    }
}
