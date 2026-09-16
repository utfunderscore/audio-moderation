use std::future::Future;

use reqwest::{Client as HttpClient, StatusCode};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;

use database::{PipelineTaskError, PipelineTaskStore};
use task_event_emitter::TaskEventEmitter;

const MODERATION_MODEL: &str = "roblox-voice-safety-v3";

#[derive(Clone)]
pub struct ModalModerationClient {
    client: HttpClient,
    endpoint: String,
    proxy_key: String,
    proxy_secret: String,
}

impl ModalModerationClient {
    pub fn new(
        client: HttpClient,
        modal_endpoint: String,
        proxy_key: String,
        proxy_secret: String,
    ) -> Self {
        Self {
            client,
            endpoint: format!("{}/moderation/", modal_endpoint.trim_end_matches('/')),
            proxy_key,
            proxy_secret,
        }
    }

    fn request(&self, request: &ModerationRequest) -> reqwest::RequestBuilder {
        self.client
            .post(&self.endpoint)
            .header("Modal-Key", &self.proxy_key)
            .header("Modal-Secret", &self.proxy_secret)
            .json(request)
    }
}

pub trait ModerationService: Send + Sync {
    fn create_moderation(
        &self,
        request: ModerationRequest,
    ) -> impl Future<Output = Result<ModerationResponse, ModerationServiceError>> + Send;
}

impl ModerationService for ModalModerationClient {
    async fn create_moderation(
        &self,
        request: ModerationRequest,
    ) -> Result<ModerationResponse, ModerationServiceError> {
        let response = self
            .request(&request)
            .send()
            .await
            .map_err(ModerationServiceError::Request)?;

        if response.status().is_success() {
            response
                .json()
                .await
                .map_err(ModerationServiceError::ResponseDecode)
        } else {
            Err(ModerationServiceError::Response(response.status()))
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModerationCallerInput {
    pub job_id: String,
    pub audio_uri: String,
    pub transcription: String,
    pub task_token: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ModerationRequest {
    model: &'static str,
    audio_uri: String,
    transcription: String,
    idempotency_key: String,
    pipeline_task_id: String,
    task_token: String,
}

impl From<ModerationCallerInput> for ModerationRequest {
    fn from(input: ModerationCallerInput) -> Self {
        Self {
            model: MODERATION_MODEL,
            audio_uri: input.audio_uri,
            transcription: input.transcription,
            idempotency_key: format!("moderation-{}", input.job_id),
            pipeline_task_id: input.job_id,
            task_token: input.task_token,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ModerationResponse {
    pub task_id: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModerationCallerOutput {
    pub job_id: String,
    pub moderation_task_id: String,
}

#[derive(Debug, Error)]
pub enum ModerationServiceError {
    #[error("moderation request failed: {0}")]
    Request(#[source] reqwest::Error),
    #[error("moderation service returned HTTP {0}")]
    Response(StatusCode),
    #[error("moderation service returned an invalid response: {0}")]
    ResponseDecode(#[source] reqwest::Error),
}

pub trait ModerationTaskStore: Send + Sync {
    fn start_moderation(
        &self,
        task_id: i32,
        task_token: String,
    ) -> impl Future<Output = Result<(), PipelineTaskError>> + Send;

    fn record_moderation_task(
        &self,
        task_id: i32,
        moderation_task_id: String,
    ) -> impl Future<Output = Result<(), PipelineTaskError>> + Send;

    fn fail_moderation_request(
        &self,
        task_id: i32,
        error_code: String,
        error_message: String,
    ) -> impl Future<Output = Result<(), PipelineTaskError>> + Send;
}

impl ModerationTaskStore for PipelineTaskStore {
    async fn start_moderation(
        &self,
        task_id: i32,
        task_token: String,
    ) -> Result<(), PipelineTaskError> {
        PipelineTaskStore::start_moderation(self, task_id, &task_token).await
    }

    async fn record_moderation_task(
        &self,
        task_id: i32,
        moderation_task_id: String,
    ) -> Result<(), PipelineTaskError> {
        PipelineTaskStore::record_moderation_task(self, task_id, &moderation_task_id).await
    }

    async fn fail_moderation_request(
        &self,
        task_id: i32,
        error_code: String,
        error_message: String,
    ) -> Result<(), PipelineTaskError> {
        PipelineTaskStore::fail_moderation_request(self, task_id, &error_code, &error_message).await
    }
}

#[derive(Debug, Error)]
pub enum ModerationCallerError {
    #[error("invalid pipeline job ID {0}")]
    InvalidJobId(String),
    #[error(transparent)]
    Service(#[from] ModerationServiceError),
    #[error("failed to start moderation processing: {0}")]
    StartModeration(#[source] PipelineTaskError),
    #[error("failed to record moderation task ID: {0}")]
    RecordModerationTask(#[source] PipelineTaskError),
    #[error("failed to record moderation request failure: {0}")]
    FailModerationRequest(#[source] PipelineTaskError),
    #[error("failed to emit task event: {0}")]
    Emit(#[source] task_event_emitter::Error),
}

#[derive(Clone)]
pub struct ModerationCallerHandler<S, D> {
    service: S,
    database: D,
    events: Option<TaskEventEmitter>,
}

impl<S, D> ModerationCallerHandler<S, D> {
    pub fn new(service: S, database: D) -> Self {
        Self {
            service,
            database,
            events: None,
        }
    }

    pub fn with_event_emitter(mut self, events: TaskEventEmitter) -> Self {
        self.events = Some(events);
        self
    }
}

impl<S: ModerationService, D: ModerationTaskStore> ModerationCallerHandler<S, D> {
    pub async fn handle(
        &self,
        input: ModerationCallerInput,
    ) -> Result<ModerationCallerOutput, ModerationCallerError> {
        let job_id = input.job_id.clone();
        let task_id = job_id
            .parse()
            .map_err(|_| ModerationCallerError::InvalidJobId(job_id.clone()))?;
        self.database
            .start_moderation(task_id, input.task_token.clone())
            .await
            .map_err(ModerationCallerError::StartModeration)?;
        self.emit(task_id, "MODERATION_PROCESSING_STARTED").await?;
        info!(jobId = job_id, "requesting moderation");
        let response = match self.service.create_moderation(input.into()).await {
            Ok(response) => response,
            Err(error) => {
                self.database
                    .fail_moderation_request(
                        task_id,
                        "MODERATION_REQUEST_FAILED".to_owned(),
                        error.to_string(),
                    )
                    .await
                    .map_err(ModerationCallerError::FailModerationRequest)?;
                self.emit(task_id, "FAILED").await?;
                return Err(error.into());
            }
        };
        self.database
            .record_moderation_task(task_id, response.task_id.clone())
            .await
            .map_err(ModerationCallerError::RecordModerationTask)?;
        Ok(ModerationCallerOutput {
            job_id,
            moderation_task_id: response.task_id,
        })
    }

    async fn emit(&self, task_id: i32, event_name: &str) -> Result<(), ModerationCallerError> {
        if let Some(events) = &self.events {
            events
                .emit(task_id, event_name)
                .await
                .map_err(ModerationCallerError::Emit)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use reqwest::header::CONTENT_TYPE;

    use super::*;

    #[derive(Default)]
    struct MockService {
        requests: Mutex<Vec<ModerationRequest>>,
        events: Option<Arc<Mutex<Vec<&'static str>>>>,
    }

    impl ModerationService for MockService {
        async fn create_moderation(
            &self,
            request: ModerationRequest,
        ) -> Result<ModerationResponse, ModerationServiceError> {
            if let Some(events) = &self.events {
                events.lock().unwrap().push("request");
            }
            self.requests.lock().unwrap().push(request);
            Ok(ModerationResponse {
                task_id: "moderation-123".to_owned(),
            })
        }
    }

    #[derive(Default)]
    struct MockDatabase {
        calls: Mutex<Vec<String>>,
        fail_start: bool,
        events: Option<Arc<Mutex<Vec<&'static str>>>>,
    }

    impl ModerationTaskStore for MockDatabase {
        async fn start_moderation(
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
                    status: database::PipelineTaskStatus::AsrFinished,
                })
            } else {
                Ok(())
            }
        }

        async fn record_moderation_task(
            &self,
            task_id: i32,
            moderation_task_id: String,
        ) -> Result<(), PipelineTaskError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("record:{task_id}:{moderation_task_id}"));
            Ok(())
        }

        async fn fail_moderation_request(
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
    async fn derives_the_moderation_request_from_transcription_output() {
        let handler = ModerationCallerHandler::new(MockService::default(), MockDatabase::default());

        assert_eq!(
            handler.handle(input()).await.unwrap(),
            ModerationCallerOutput {
                job_id: "42".to_owned(),
                moderation_task_id: "moderation-123".to_owned(),
            }
        );
        assert_eq!(
            *handler.service.requests.lock().unwrap(),
            vec![ModerationRequest {
                model: MODERATION_MODEL,
                audio_uri: "s3://audio-bucket/evaluations/42/audio.wav".to_owned(),
                transcription: "hello world".to_owned(),
                idempotency_key: "moderation-42".to_owned(),
                pipeline_task_id: "42".to_owned(),
                task_token: "step-functions-token".to_owned(),
            }]
        );
        assert_eq!(
            *handler.database.calls.lock().unwrap(),
            ["start:42", "record:42:moderation-123"]
        );
    }

    #[tokio::test]
    async fn does_not_request_moderation_when_starting_moderation_fails() {
        let handler = ModerationCallerHandler::new(
            MockService::default(),
            MockDatabase {
                fail_start: true,
                ..Default::default()
            },
        );

        assert!(matches!(
            handler.handle(input()).await,
            Err(ModerationCallerError::StartModeration(_))
        ));
        assert!(handler.service.requests.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn starts_moderation_before_requesting_it() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let handler = ModerationCallerHandler::new(
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
    fn builds_the_modal_request_as_expected() {
        let client = ModalModerationClient::new(
            HttpClient::new(),
            "https://example.modal.run/".to_owned(),
            "proxy-key".to_owned(),
            "proxy-secret".to_owned(),
        );
        let request = client
            .request(&ModerationRequest::from(input()))
            .build()
            .unwrap();

        assert_eq!(request.method(), reqwest::Method::POST);
        assert_eq!(
            request.url().as_str(),
            "https://example.modal.run/moderation/"
        );
        assert_eq!(request.headers()["Modal-Key"], "proxy-key");
        assert_eq!(request.headers()["Modal-Secret"], "proxy-secret");
        assert_eq!(request.headers()[CONTENT_TYPE], "application/json");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(
                request.body().unwrap().as_bytes().unwrap()
            )
            .unwrap(),
            serde_json::json!({
                "model": "roblox-voice-safety-v3",
                "audio_uri": "s3://audio-bucket/evaluations/42/audio.wav",
                "transcription": "hello world",
                "idempotency_key": "moderation-42",
                "pipeline_task_id": "42",
                "task_token": "step-functions-token",
            })
        );
    }

    fn input() -> ModerationCallerInput {
        ModerationCallerInput {
            job_id: "42".to_owned(),
            audio_uri: "s3://audio-bucket/evaluations/42/audio.wav".to_owned(),
            transcription: "hello world".to_owned(),
            task_token: "step-functions-token".to_owned(),
        }
    }
}
