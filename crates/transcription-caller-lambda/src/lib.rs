use std::future::Future;

use reqwest::{Client as HttpClient, StatusCode};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;

use database::{PipelineTaskError, PipelineTaskStore};

const TRANSCRIPTION_MODEL: &str = "granite";

#[derive(Clone)]
pub struct ModalAsrClient {
    client: HttpClient,
    endpoint: String,
    token: String,
}

impl ModalAsrClient {
    pub fn new(client: HttpClient, endpoint: String, token: String) -> Self {
        Self {
            client,
            endpoint,
            token,
        }
    }
}

pub trait TranscriptionService: Send + Sync {
    fn create_transcription(
        &self,
        request: TranscriptionRequest,
    ) -> impl Future<Output = Result<TranscriptionResponse, TranscriptionServiceError>> + Send;
}

impl TranscriptionService for ModalAsrClient {
    async fn create_transcription(
        &self,
        request: TranscriptionRequest,
    ) -> Result<TranscriptionResponse, TranscriptionServiceError> {
        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.token)
            .json(&request)
            .send()
            .await
            .map_err(TranscriptionServiceError::Request)?;

        if response.status().is_success() {
            response
                .json()
                .await
                .map_err(TranscriptionServiceError::ResponseDecode)
        } else {
            Err(TranscriptionServiceError::Response(response.status()))
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionCallerInput {
    pub job_id: String,
    pub audio_uri: String,
    pub task_token: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct TranscriptionRequest {
    model: &'static str,
    audio_uri: String,
    idempotency_key: String,
    pipeline_task_id: String,
    task_token: String,
}

impl From<TranscriptionCallerInput> for TranscriptionRequest {
    fn from(input: TranscriptionCallerInput) -> Self {
        Self {
            model: TRANSCRIPTION_MODEL,
            audio_uri: input.audio_uri,
            idempotency_key: format!("transcription-{}", input.job_id),
            pipeline_task_id: input.job_id,
            task_token: input.task_token,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct TranscriptionResponse {
    pub task_id: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionCallerOutput {
    pub job_id: String,
    pub asr_task_id: String,
}

#[derive(Debug, Error)]
pub enum TranscriptionServiceError {
    #[error("ASR request failed: {0}")]
    Request(#[source] reqwest::Error),
    #[error("ASR service returned HTTP {0}")]
    Response(StatusCode),
    #[error("ASR service returned an invalid response: {0}")]
    ResponseDecode(#[source] reqwest::Error),
}

pub trait AsrTaskStore: Send + Sync {
    fn start_asr(
        &self,
        task_id: i32,
        task_token: String,
    ) -> impl Future<Output = Result<(), PipelineTaskError>> + Send;

    fn record_asr_task(
        &self,
        task_id: i32,
        asr_task_id: String,
    ) -> impl Future<Output = Result<(), PipelineTaskError>> + Send;

    fn fail_asr_request(
        &self,
        task_id: i32,
        error_code: String,
        error_message: String,
    ) -> impl Future<Output = Result<(), PipelineTaskError>> + Send;
}

impl AsrTaskStore for PipelineTaskStore {
    async fn start_asr(&self, task_id: i32, task_token: String) -> Result<(), PipelineTaskError> {
        PipelineTaskStore::start_asr(self, task_id, &task_token).await
    }

    async fn record_asr_task(
        &self,
        task_id: i32,
        asr_task_id: String,
    ) -> Result<(), PipelineTaskError> {
        PipelineTaskStore::record_asr_task(self, task_id, &asr_task_id).await
    }

    async fn fail_asr_request(
        &self,
        task_id: i32,
        error_code: String,
        error_message: String,
    ) -> Result<(), PipelineTaskError> {
        PipelineTaskStore::fail_asr_request(self, task_id, &error_code, &error_message).await
    }
}

#[derive(Debug, Error)]
pub enum TranscriptionCallerError {
    #[error("invalid pipeline job ID {0}")]
    InvalidJobId(String),
    #[error(transparent)]
    Service(#[from] TranscriptionServiceError),
    #[error("failed to start ASR processing: {0}")]
    StartAsr(#[source] PipelineTaskError),
    #[error("failed to record ASR task ID: {0}")]
    RecordAsrTask(#[source] PipelineTaskError),
    #[error("failed to record ASR request failure: {0}")]
    FailAsrRequest(#[source] PipelineTaskError),
}

#[derive(Clone)]
pub struct TranscriptionCallerHandler<S, D> {
    service: S,
    database: D,
}

impl<S, D> TranscriptionCallerHandler<S, D> {
    pub fn new(service: S, database: D) -> Self {
        Self { service, database }
    }
}

impl<S: TranscriptionService, D: AsrTaskStore> TranscriptionCallerHandler<S, D> {
    pub async fn handle(
        &self,
        input: TranscriptionCallerInput,
    ) -> Result<TranscriptionCallerOutput, TranscriptionCallerError> {
        let job_id = input.job_id.clone();
        let task_id = job_id
            .parse()
            .map_err(|_| TranscriptionCallerError::InvalidJobId(job_id.clone()))?;
        self.database
            .start_asr(task_id, input.task_token.clone())
            .await
            .map_err(TranscriptionCallerError::StartAsr)?;
        info!(jobId = job_id, "requesting transcription");
        let response = match self.service.create_transcription(input.into()).await {
            Ok(response) => response,
            Err(error) => {
                self.database
                    .fail_asr_request(task_id, "ASR_REQUEST_FAILED".to_owned(), error.to_string())
                    .await
                    .map_err(TranscriptionCallerError::FailAsrRequest)?;
                return Err(error.into());
            }
        };
        self.database
            .record_asr_task(task_id, response.task_id.clone())
            .await
            .map_err(TranscriptionCallerError::RecordAsrTask)?;
        Ok(TranscriptionCallerOutput {
            job_id,
            asr_task_id: response.task_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Default)]
    struct MockService {
        requests: Mutex<Vec<TranscriptionRequest>>,
        events: Option<Arc<Mutex<Vec<&'static str>>>>,
    }

    impl TranscriptionService for MockService {
        async fn create_transcription(
            &self,
            request: TranscriptionRequest,
        ) -> Result<TranscriptionResponse, TranscriptionServiceError> {
            if let Some(events) = &self.events {
                events.lock().unwrap().push("request");
            }
            self.requests.lock().unwrap().push(request);
            Ok(TranscriptionResponse {
                task_id: "fc-123".to_owned(),
            })
        }
    }

    #[derive(Default)]
    struct MockDatabase {
        calls: Mutex<Vec<String>>,
        fail_start: bool,
        events: Option<Arc<Mutex<Vec<&'static str>>>>,
    }

    impl AsrTaskStore for MockDatabase {
        async fn start_asr(
            &self,
            task_id: i32,
            _task_token: String,
        ) -> Result<(), PipelineTaskError> {
            self.calls.lock().unwrap().push(format!("start:{task_id}"));
            if let Some(events) = &self.events {
                events.lock().unwrap().push("start");
            }
            if self.fail_start {
                Err(PipelineTaskError::TransitionConflict {
                    task_id,
                    status: database::PipelineTaskStatus::Pending,
                })
            } else {
                Ok(())
            }
        }

        async fn record_asr_task(
            &self,
            task_id: i32,
            asr_task_id: String,
        ) -> Result<(), PipelineTaskError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("record:{task_id}:{asr_task_id}"));
            Ok(())
        }

        async fn fail_asr_request(
            &self,
            task_id: i32,
            _error_code: String,
            _error_message: String,
        ) -> Result<(), PipelineTaskError> {
            self.calls.lock().unwrap().push(format!("fail:{task_id}"));
            Ok(())
        }
    }

    #[tokio::test]
    async fn derives_the_asr_request_from_pipeline_output() {
        let handler =
            TranscriptionCallerHandler::new(MockService::default(), MockDatabase::default());
        let input = input();

        assert_eq!(
            handler.handle(input).await.unwrap(),
            TranscriptionCallerOutput {
                job_id: "42".to_owned(),
                asr_task_id: "fc-123".to_owned(),
            }
        );
        assert_eq!(
            *handler.service.requests.lock().unwrap(),
            vec![TranscriptionRequest {
                model: TRANSCRIPTION_MODEL,
                audio_uri: "s3://audio-bucket/evaluations/42/audio.wav".to_owned(),
                idempotency_key: "transcription-42".to_owned(),
                pipeline_task_id: "42".to_owned(),
                task_token: "step-functions-token".to_owned(),
            }]
        );
        assert_eq!(
            *handler.database.calls.lock().unwrap(),
            vec!["start:42", "record:42:fc-123"]
        );
    }

    #[tokio::test]
    async fn does_not_request_transcription_when_starting_asr_fails() {
        let handler = TranscriptionCallerHandler::new(
            MockService::default(),
            MockDatabase {
                fail_start: true,
                ..Default::default()
            },
        );

        assert!(matches!(
            handler.handle(input()).await,
            Err(TranscriptionCallerError::StartAsr(_))
        ));
        assert!(handler.service.requests.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn starts_asr_before_requesting_transcription() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let handler = TranscriptionCallerHandler::new(
            MockService {
                events: Some(Arc::clone(&events)),
                ..Default::default()
            },
            MockDatabase {
                events: Some(Arc::clone(&events)),
                ..Default::default()
            },
        );

        handler.handle(input()).await.unwrap();
        assert_eq!(*events.lock().unwrap(), ["start", "request"]);
    }

    #[test]
    fn serializes_the_asr_request_as_expected() {
        let request = TranscriptionRequest::from(input());

        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::json!({
                "model": "granite",
                "audio_uri": "s3://audio-bucket/evaluations/42/audio.wav",
                "idempotency_key": "transcription-42",
                "pipeline_task_id": "42",
                "task_token": "step-functions-token",
            })
        );
    }

    fn input() -> TranscriptionCallerInput {
        TranscriptionCallerInput {
            job_id: "42".to_owned(),
            audio_uri: "s3://audio-bucket/evaluations/42/audio.wav".to_owned(),
            task_token: "step-functions-token".to_owned(),
        }
    }
}
