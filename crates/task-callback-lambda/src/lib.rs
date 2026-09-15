use std::collections::HashMap;
use std::future::Future;

use aws_sdk_sfn::Client as SfnClient;
use aws_sdk_sfn::error::{ProvideErrorMetadata, SdkError};
use lambda_http::Body;
use serde::{Deserialize, Serialize};
use tracing::info;

use database::{CallbackAttempt, ModerationResult, PipelineTaskError, PipelineTaskStore};

const MAX_TASK_TOKEN_BYTES: usize = 2_048;
const MAX_SUCCESS_OUTPUT_BYTES: usize = 262_144;
const MAX_ERROR_BYTES: usize = 256;
const MAX_CAUSE_BYTES: usize = 32_768;

#[derive(Clone)]
pub struct AwsStepFunctions {
    client: SfnClient,
}

impl AwsStepFunctions {
    pub fn new(client: SfnClient) -> Self {
        Self { client }
    }
}

pub trait StepFunctions: Send + Sync {
    fn send_task_success(
        &self,
        task_token: String,
        output: String,
    ) -> impl Future<Output = Result<(), StepFunctionsError>> + Send;

    fn send_task_failure(
        &self,
        task_token: String,
        error: Option<String>,
        cause: Option<String>,
    ) -> impl Future<Output = Result<(), StepFunctionsError>> + Send;
}

/// Persistence needed to accept external task callbacks.
pub trait TaskCallbackStore: Send + Sync {
    fn complete_asr(
        &self,
        task_id: i32,
        task_token: String,
        asr_task_id: String,
        transcription: String,
    ) -> impl Future<Output = Result<(), PipelineTaskError>> + Send;

    fn complete_moderation(
        &self,
        task_token: String,
        job_id: i32,
        moderation_task_id: String,
        result: ModerationResult,
    ) -> impl Future<Output = Result<(), PipelineTaskError>> + Send;

    fn record_callback_failure(
        &self,
        task_token: String,
        error: Option<String>,
        cause: Option<String>,
    ) -> impl Future<Output = Result<CallbackAttempt, PipelineTaskError>> + Send;

    fn finalize_callback_failure(
        &self,
        task_token: String,
    ) -> impl Future<Output = Result<(), PipelineTaskError>> + Send;
}

impl TaskCallbackStore for PipelineTaskStore {
    async fn complete_asr(
        &self,
        task_id: i32,
        task_token: String,
        asr_task_id: String,
        transcription: String,
    ) -> Result<(), PipelineTaskError> {
        PipelineTaskStore::complete_asr(self, task_id, &task_token, &asr_task_id, &transcription)
            .await
    }

    async fn complete_moderation(
        &self,
        task_token: String,
        job_id: i32,
        moderation_task_id: String,
        result: ModerationResult,
    ) -> Result<(), PipelineTaskError> {
        PipelineTaskStore::complete_moderation_for_callback(
            self,
            &task_token,
            job_id,
            &moderation_task_id,
            result,
        )
        .await
    }

    async fn record_callback_failure(
        &self,
        task_token: String,
        error: Option<String>,
        cause: Option<String>,
    ) -> Result<CallbackAttempt, PipelineTaskError> {
        PipelineTaskStore::record_callback_failure(
            self,
            &task_token,
            error.as_deref(),
            cause.as_deref(),
        )
        .await
    }

    async fn finalize_callback_failure(&self, task_token: String) -> Result<(), PipelineTaskError> {
        PipelineTaskStore::finalize_callback_failure(self, &task_token).await
    }
}

impl StepFunctions for AwsStepFunctions {
    async fn send_task_success(
        &self,
        task_token: String,
        output: String,
    ) -> Result<(), StepFunctionsError> {
        self.client
            .send_task_success()
            .task_token(task_token)
            .output(output)
            .send()
            .await
            .map(|_| ())
            .map_err(classify_sdk_error)
    }

    async fn send_task_failure(
        &self,
        task_token: String,
        error: Option<String>,
        cause: Option<String>,
    ) -> Result<(), StepFunctionsError> {
        self.client
            .send_task_failure()
            .task_token(task_token)
            .set_error(error)
            .set_cause(cause)
            .send()
            .await
            .map(|_| ())
            .map_err(classify_sdk_error)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepFunctionsError {
    InvalidToken,
    TaskTimedOut,
    TaskDoesNotExist,
    Unavailable,
    Other,
}

fn classify_sdk_error<E>(error: SdkError<E>) -> StepFunctionsError
where
    E: ProvideErrorMetadata,
{
    match error
        .as_service_error()
        .and_then(ProvideErrorMetadata::code)
    {
        Some("InvalidToken") => return StepFunctionsError::InvalidToken,
        Some("TaskTimedOut") => return StepFunctionsError::TaskTimedOut,
        Some("TaskDoesNotExist") => return StepFunctionsError::TaskDoesNotExist,
        Some("KmsThrottlingException") => return StepFunctionsError::Unavailable,
        _ => {}
    }

    match error {
        SdkError::TimeoutError(_) | SdkError::DispatchFailure(_) | SdkError::ResponseError(_) => {
            StepFunctionsError::Unavailable
        }
        _ => StepFunctionsError::Other,
    }
}

#[derive(Clone)]
pub struct TaskCallbackHandler<S, D> {
    step_functions: S,
    database: D,
}

impl<S, D> TaskCallbackHandler<S, D> {
    pub fn new(step_functions: S, database: D) -> Self {
        Self {
            step_functions,
            database,
        }
    }
}

impl<S: StepFunctions, D: TaskCallbackStore> TaskCallbackHandler<S, D> {
    pub async fn handle(&self, body: &[u8]) -> u16 {
        let callback = match serde_json::from_slice::<CallbackRequest>(body) {
            Ok(callback) => callback,
            Err(_) => return 400,
        };

        if !(1..=MAX_TASK_TOKEN_BYTES).contains(&callback.task_token.len()) {
            return 400;
        }

        let result = match callback.outcome {
            Outcome::Success {
                payload:
                    SuccessPayload::Transcription {
                        transcription_result,
                    },
            } => {
                let output = match serde_json::to_string(&transcription_result) {
                    Ok(output) if output.len() <= MAX_SUCCESS_OUTPUT_BYTES => output,
                    _ => return 400,
                };
                let task_id = match transcription_result.job_id.parse() {
                    Ok(task_id) => task_id,
                    Err(_) => return 400,
                };
                info!(outcome = "success", "received external task callback");
                match self
                    .database
                    .complete_asr(
                        task_id,
                        callback.task_token.clone(),
                        transcription_result.asr_task_id.clone(),
                        transcription_result.transcription.clone(),
                    )
                    .await
                {
                    Ok(()) => {}
                    Err(
                        PipelineTaskError::AsrTaskConflict
                        | PipelineTaskError::TranscriptionConflict
                        | PipelineTaskError::CallbackTokenNotFound
                        | PipelineTaskError::CallbackTaskConflict
                        | PipelineTaskError::CallbackStepConflict
                        | PipelineTaskError::TransitionConflict { .. },
                    ) => return 409,
                    Err(_) => return 500,
                }
                match self
                    .step_functions
                    .send_task_success(callback.task_token.clone(), output)
                    .await
                {
                    // The state machine advances to moderation only after the
                    // transcription result is durably stored.
                    Ok(()) => Ok(()),
                    Err(error) => Err(error),
                }
            }
            Outcome::Success {
                payload: SuccessPayload::Moderation { moderation_result },
            } => {
                let result = match validate_moderation_result(&moderation_result.scores) {
                    Ok(result) => result,
                    Err(()) => return 400,
                };
                let output = match serde_json::to_string(&moderation_result.scores) {
                    Ok(output) if output.len() <= MAX_SUCCESS_OUTPUT_BYTES => output,
                    _ => return 400,
                };
                let job_id = match moderation_result.job_id.parse() {
                    Ok(job_id) => job_id,
                    Err(_) => return 400,
                };
                info!(outcome = "success", "received external task callback");
                match self
                    .database
                    .complete_moderation(
                        callback.task_token.clone(),
                        job_id,
                        moderation_result.moderation_task_id,
                        result,
                    )
                    .await
                {
                    Ok(()) => {}
                    Err(PipelineTaskError::ModerationJobConflict) => {
                        return self
                            .handle_failure(
                                callback.task_token,
                                Some("ModerationJobMismatch".to_owned()),
                                Some(format!(
                                    "moderation callback jobId {job_id} does not match the task for its taskToken"
                                )),
                            )
                            .await;
                    }
                    Err(
                        PipelineTaskError::ModerationConflict
                        | PipelineTaskError::CallbackTokenNotFound
                        | PipelineTaskError::CallbackTaskConflict
                        | PipelineTaskError::CallbackStepConflict
                        | PipelineTaskError::TransitionConflict { .. },
                    ) => return 409,
                    Err(_) => return 500,
                }
                self.step_functions
                    .send_task_success(callback.task_token.clone(), output)
                    .await
            }
            Outcome::Failure { error, cause } => {
                if error
                    .as_ref()
                    .is_some_and(|value| value.len() > MAX_ERROR_BYTES)
                    || cause
                        .as_ref()
                        .is_some_and(|value| value.len() > MAX_CAUSE_BYTES)
                {
                    return 400;
                }
                return self.handle_failure(callback.task_token, error, cause).await;
            }
        };

        match result {
            Ok(()) => 204,
            Err(
                StepFunctionsError::InvalidToken
                | StepFunctionsError::TaskTimedOut
                | StepFunctionsError::TaskDoesNotExist,
            ) => 409,
            Err(StepFunctionsError::Unavailable) => 503,
            Err(StepFunctionsError::Other) => 502,
        }
    }

    async fn handle_failure(
        &self,
        task_token: String,
        error: Option<String>,
        cause: Option<String>,
    ) -> u16 {
        info!(outcome = "failure", "received external task callback");
        match self
            .database
            .record_callback_failure(task_token.clone(), error.clone(), cause.clone())
            .await
        {
            Ok(_) => {}
            Err(
                PipelineTaskError::CallbackTokenNotFound
                | PipelineTaskError::CallbackTaskConflict
                | PipelineTaskError::CallbackStepConflict
                | PipelineTaskError::TranscriptionConflict
                | PipelineTaskError::ModerationConflict
                | PipelineTaskError::TransitionConflict { .. },
            ) => return 409,
            Err(_) => return 500,
        }
        match self
            .step_functions
            .send_task_failure(task_token.clone(), error, cause)
            .await
        {
            Ok(()) => match self.database.finalize_callback_failure(task_token).await {
                Ok(()) => 204,
                Err(
                    PipelineTaskError::CallbackTokenNotFound
                    | PipelineTaskError::CallbackTaskConflict
                    | PipelineTaskError::CallbackStepConflict
                    | PipelineTaskError::TransitionConflict { .. },
                ) => 409,
                Err(_) => 502,
            },
            Err(
                StepFunctionsError::InvalidToken
                | StepFunctionsError::TaskTimedOut
                | StepFunctionsError::TaskDoesNotExist,
            ) => 409,
            Err(StepFunctionsError::Unavailable) => 503,
            Err(StepFunctionsError::Other) => 502,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CallbackRequest {
    task_token: String,
    outcome: Outcome,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum Outcome {
    #[serde(rename = "success")]
    Success {
        #[serde(flatten)]
        payload: SuccessPayload,
    },
    #[serde(rename = "failure")]
    Failure {
        error: Option<String>,
        cause: Option<String>,
    },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SuccessPayload {
    Transcription {
        #[serde(rename = "transcriptionResult")]
        transcription_result: TranscriptionResult,
    },
    Moderation {
        #[serde(rename = "moderationResult")]
        moderation_result: ModerationCallbackResult,
    },
}

fn validate_moderation_result(scores: &HashMap<String, f64>) -> Result<ModerationResult, ()> {
    const CATEGORIES: [&str; 5] = [
        "sexual",
        "hate_or_discrimination",
        "harassment_or_abuse",
        "violence_or_threats",
        "asking_for_pii",
    ];
    if scores.len() != CATEGORIES.len()
        || !CATEGORIES
            .iter()
            .all(|category| scores.contains_key(*category))
    {
        return Err(());
    }
    let score = |category: &str| scores[category];
    let result = ModerationResult {
        sexual: score("sexual"),
        hate_or_discrimination: score("hate_or_discrimination"),
        harassment_or_abuse: score("harassment_or_abuse"),
        violence_or_threats: score("violence_or_threats"),
        asking_for_pii: score("asking_for_pii"),
    };
    if [
        result.sexual,
        result.hate_or_discrimination,
        result.harassment_or_abuse,
        result.violence_or_threats,
        result.asking_for_pii,
    ]
    .iter()
    .all(|score| score.is_finite() && (0.0..=1.0).contains(score))
    {
        Ok(result)
    } else {
        Err(())
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct TranscriptionResult {
    job_id: String,
    asr_task_id: String,
    transcription: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModerationCallbackResult {
    job_id: String,
    moderation_task_id: String,
    scores: HashMap<String, f64>,
}

pub fn response(status: u16) -> lambda_http::Response<Body> {
    lambda_http::Response::builder()
        .status(status)
        .body(Body::Empty)
        .expect("callback response status is valid")
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    struct MockStepFunctions {
        calls: Mutex<Vec<Call>>,
        result: Mutex<Result<(), StepFunctionsError>>,
        events: Option<Arc<Mutex<Vec<&'static str>>>>,
    }

    struct MockDatabase {
        calls: Mutex<Vec<(i32, String, String)>>,
        moderation_calls: Mutex<Vec<(i32, String, ModerationResult)>>,
        failure_calls: Mutex<Vec<(Option<String>, Option<String>)>>,
        result: Mutex<MockDatabaseResult>,
        failure_result: Mutex<MockFailureResult>,
        events: Option<Arc<Mutex<Vec<&'static str>>>>,
    }

    #[derive(Clone, Copy, Default)]
    enum MockDatabaseResult {
        #[default]
        Ok,
        Conflict,
        ModerationJobMismatch,
        TransitionConflict,
        Failure,
    }

    #[derive(Clone, Copy, Default)]
    enum MockFailureResult {
        #[default]
        Ok,
        PersistenceTransitionConflict,
        FinalizationTransitionConflict,
    }

    impl Default for MockDatabase {
        fn default() -> Self {
            Self {
                calls: Mutex::default(),
                moderation_calls: Mutex::default(),
                failure_calls: Mutex::default(),
                result: Mutex::new(MockDatabaseResult::Ok),
                failure_result: Mutex::new(MockFailureResult::Ok),
                events: None,
            }
        }
    }

    impl MockDatabase {
        fn calls(&self) -> Vec<(i32, String, String)> {
            self.calls.lock().unwrap().clone()
        }

        fn set_result(&self, result: MockDatabaseResult) {
            *self.result.lock().unwrap() = result;
        }

        fn moderation_calls(&self) -> Vec<(i32, String, ModerationResult)> {
            self.moderation_calls.lock().unwrap().clone()
        }

        fn failure_calls(&self) -> Vec<(Option<String>, Option<String>)> {
            self.failure_calls.lock().unwrap().clone()
        }

        fn set_failure_result(&self, result: MockFailureResult) {
            *self.failure_result.lock().unwrap() = result;
        }
    }

    impl TaskCallbackStore for MockDatabase {
        async fn complete_asr(
            &self,
            task_id: i32,
            _task_token: String,
            asr_task_id: String,
            transcription: String,
        ) -> Result<(), PipelineTaskError> {
            if let Some(events) = &self.events {
                events.lock().unwrap().push("complete-asr");
            }
            self.calls
                .lock()
                .unwrap()
                .push((task_id, asr_task_id, transcription));
            match *self.result.lock().unwrap() {
                MockDatabaseResult::Ok => Ok(()),
                MockDatabaseResult::Conflict => Err(PipelineTaskError::TranscriptionConflict),
                MockDatabaseResult::ModerationJobMismatch => {
                    Err(PipelineTaskError::Lifecycle(sqlx::Error::PoolClosed))
                }
                MockDatabaseResult::TransitionConflict => Err(transition_conflict()),
                MockDatabaseResult::Failure => {
                    Err(PipelineTaskError::Lifecycle(sqlx::Error::PoolClosed))
                }
            }
        }

        async fn complete_moderation(
            &self,
            _task_token: String,
            job_id: i32,
            moderation_task_id: String,
            result: ModerationResult,
        ) -> Result<(), PipelineTaskError> {
            if let Some(events) = &self.events {
                events.lock().unwrap().push("complete-moderation");
            }
            self.moderation_calls
                .lock()
                .unwrap()
                .push((job_id, moderation_task_id, result));
            match *self.result.lock().unwrap() {
                MockDatabaseResult::Ok => Ok(()),
                MockDatabaseResult::Conflict => Err(PipelineTaskError::ModerationConflict),
                MockDatabaseResult::ModerationJobMismatch => {
                    Err(PipelineTaskError::ModerationJobConflict)
                }
                MockDatabaseResult::TransitionConflict => Err(transition_conflict()),
                MockDatabaseResult::Failure => {
                    Err(PipelineTaskError::Lifecycle(sqlx::Error::PoolClosed))
                }
            }
        }

        async fn record_callback_failure(
            &self,
            _task_token: String,
            error: Option<String>,
            cause: Option<String>,
        ) -> Result<CallbackAttempt, PipelineTaskError> {
            if let Some(events) = &self.events {
                events.lock().unwrap().push("record-failure");
            }
            self.failure_calls.lock().unwrap().push((error, cause));
            match *self.failure_result.lock().unwrap() {
                MockFailureResult::PersistenceTransitionConflict => Err(transition_conflict()),
                MockFailureResult::Ok | MockFailureResult::FinalizationTransitionConflict => {
                    Ok(CallbackAttempt {
                        task_id: 42,
                        step: database::PipelineCallbackStep::Transcription,
                    })
                }
            }
        }

        async fn finalize_callback_failure(
            &self,
            _task_token: String,
        ) -> Result<(), PipelineTaskError> {
            if let Some(events) = &self.events {
                events.lock().unwrap().push("finalize-failure");
            }
            match *self.failure_result.lock().unwrap() {
                MockFailureResult::FinalizationTransitionConflict => Err(transition_conflict()),
                MockFailureResult::Ok | MockFailureResult::PersistenceTransitionConflict => Ok(()),
            }
        }
    }

    fn transition_conflict() -> PipelineTaskError {
        PipelineTaskError::TransitionConflict {
            task_id: 42,
            status: database::PipelineTaskStatus::StartedAsr,
        }
    }

    impl Default for MockStepFunctions {
        fn default() -> Self {
            Self {
                calls: Mutex::default(),
                result: Mutex::new(Ok(())),
                events: None,
            }
        }
    }

    #[derive(Clone, Debug, PartialEq)]
    enum Call {
        Success {
            task_token: String,
            output: String,
        },
        Failure {
            task_token: String,
            error: Option<String>,
            cause: Option<String>,
        },
    }

    impl MockStepFunctions {
        fn calls(&self) -> Vec<Call> {
            self.calls.lock().unwrap().clone()
        }

        fn set_result(&self, result: Result<(), StepFunctionsError>) {
            *self.result.lock().unwrap() = result;
        }
    }

    impl StepFunctions for MockStepFunctions {
        async fn send_task_success(
            &self,
            task_token: String,
            output: String,
        ) -> Result<(), StepFunctionsError> {
            if let Some(events) = &self.events {
                events.lock().unwrap().push("send-success");
            }
            self.calls
                .lock()
                .unwrap()
                .push(Call::Success { task_token, output });
            *self.result.lock().unwrap()
        }

        async fn send_task_failure(
            &self,
            task_token: String,
            error: Option<String>,
            cause: Option<String>,
        ) -> Result<(), StepFunctionsError> {
            if let Some(events) = &self.events {
                events.lock().unwrap().push("send-failure");
            }
            self.calls.lock().unwrap().push(Call::Failure {
                task_token,
                error,
                cause,
            });
            *self.result.lock().unwrap()
        }
    }

    #[tokio::test]
    async fn forwards_transcription_result_without_the_task_token() {
        let step_functions = MockStepFunctions::default();
        let handler = TaskCallbackHandler::new(step_functions, MockDatabase::default());
        let body = br#"{"taskToken":"token-123","outcome":{"type":"success","transcriptionResult":{"jobId":"42","asrTaskId":"fc-123","transcription":"hello world"}}}"#;

        assert_eq!(handler.handle(body).await, 204);
        assert_eq!(
            handler.step_functions.calls(),
            vec![Call::Success {
                task_token: "token-123".to_owned(),
                output: r#"{"jobId":"42","asrTaskId":"fc-123","transcription":"hello world"}"#
                    .to_owned(),
            }]
        );
        assert_eq!(
            handler.database.calls(),
            vec![(42, "fc-123".to_owned(), "hello world".to_owned())]
        );
    }

    #[tokio::test]
    async fn persists_moderation_before_sending_success() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let handler = TaskCallbackHandler::new(
            MockStepFunctions {
                events: Some(Arc::clone(&events)),
                ..Default::default()
            },
            MockDatabase {
                events: Some(Arc::clone(&events)),
                ..Default::default()
            },
        );

        assert_eq!(
            handler
                .handle(&moderation_success_body("token", scores()))
                .await,
            204
        );
        assert_eq!(
            *events.lock().unwrap(),
            ["complete-moderation", "send-success"]
        );
        assert_eq!(
            handler.database.moderation_calls(),
            vec![(
                42,
                "moderation_0123456789abcdef0123456789abcdef".to_owned(),
                ModerationResult {
                    sexual: 0.02,
                    hate_or_discrimination: 0.15,
                    harassment_or_abuse: 0.08,
                    violence_or_threats: 0.01,
                    asking_for_pii: 0.42,
                },
            ),]
        );
        let calls = handler.step_functions.calls();
        assert!(matches!(
            calls.as_slice(),
            [Call::Success { task_token, .. }] if task_token == "token"
        ));
        let Call::Success { output, .. } = &calls[0] else {
            unreachable!();
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(output).unwrap(),
            serde_json::to_value(scores()).unwrap()
        );
    }

    #[tokio::test]
    async fn rejects_moderation_scores_without_exact_categories_or_valid_ranges() {
        let handler =
            TaskCallbackHandler::new(MockStepFunctions::default(), MockDatabase::default());
        for result in [
            serde_json::json!({
                "sexual": 0.02,
                "hate_or_discrimination": 0.15,
                "harassment_or_abuse": 0.08,
                "violence_or_threats": 0.01,
            }),
            serde_json::json!({
                "sexual": 0.02,
                "hate_or_discrimination": 0.15,
                "harassment_or_abuse": 0.08,
                "violence_or_threats": 0.01,
                "asking_for_pii": 0.42,
                "unknown": 0.2,
            }),
            serde_json::json!({
                "sexual": -0.01,
                "hate_or_discrimination": 0.15,
                "harassment_or_abuse": 0.08,
                "violence_or_threats": 0.01,
                "asking_for_pii": 0.42,
            }),
            serde_json::json!({
                "sexual": 0.02,
                "hate_or_discrimination": 0.15,
                "harassment_or_abuse": 0.08,
                "violence_or_threats": 1.01,
                "asking_for_pii": 0.42,
            }),
        ] {
            let body = serde_json::to_vec(&serde_json::json!({
                "taskToken": "token",
                "outcome": {
                    "type": "success",
                    "moderationResult": {
                        "jobId": "42",
                        "moderationTaskId": "moderation_0123456789abcdef0123456789abcdef",
                        "scores": result,
                    },
                },
            }))
            .unwrap();
            assert_eq!(handler.handle(&body).await, 400);
        }
        assert_eq!(handler.handle(br#"{"taskToken":"token","outcome":{"type":"success","moderationResult":{"jobId":"42","moderationTaskId":"moderation_0123456789abcdef0123456789abcdef","scores":{"sexual":1e999,"hate_or_discrimination":0.15,"harassment_or_abuse":0.08,"violence_or_threats":0.01,"asking_for_pii":0.42}}}}"#).await, 400);
        assert!(handler.database.moderation_calls().is_empty());
        assert!(handler.step_functions.calls().is_empty());
    }

    #[test]
    fn rejects_non_finite_moderation_scores() {
        let mut values = scores();
        values.insert("sexual".to_owned(), f64::INFINITY);
        assert!(validate_moderation_result(&values).is_err());
    }

    #[tokio::test]
    async fn rejects_a_conflicting_moderation_retry_without_sending_success() {
        let database = MockDatabase::default();
        database.set_result(MockDatabaseResult::Conflict);
        let handler = TaskCallbackHandler::new(MockStepFunctions::default(), database);

        assert_eq!(
            handler
                .handle(&moderation_success_body("token", scores()))
                .await,
            409
        );
        assert!(handler.step_functions.calls().is_empty());
    }

    #[tokio::test]
    async fn rejects_a_non_numeric_moderation_job_id() {
        let handler =
            TaskCallbackHandler::new(MockStepFunctions::default(), MockDatabase::default());
        let body = serde_json::to_vec(&serde_json::json!({
            "taskToken": "token",
            "outcome": {
                "type": "success",
                "moderationResult": {
                    "jobId": "not-a-number",
                    "moderationTaskId": "moderation_0123456789abcdef0123456789abcdef",
                    "scores": scores(),
                },
            },
        }))
        .unwrap();

        assert_eq!(handler.handle(&body).await, 400);
        assert!(handler.database.moderation_calls().is_empty());
        assert!(handler.step_functions.calls().is_empty());
    }

    #[tokio::test]
    async fn fails_a_moderation_job_id_mismatch_and_finalizes_the_callback() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let database = MockDatabase {
            events: Some(Arc::clone(&events)),
            ..Default::default()
        };
        database.set_result(MockDatabaseResult::ModerationJobMismatch);
        let handler = TaskCallbackHandler::new(
            MockStepFunctions {
                events: Some(Arc::clone(&events)),
                ..Default::default()
            },
            database,
        );

        assert_eq!(
            handler
                .handle(&moderation_success_body("token", scores()))
                .await,
            204
        );
        assert_eq!(
            *events.lock().unwrap(),
            [
                "complete-moderation",
                "record-failure",
                "send-failure",
                "finalize-failure",
            ]
        );
        assert_eq!(
            handler.database.failure_calls(),
            vec![(
                Some("ModerationJobMismatch".to_owned()),
                Some(
                    "moderation callback jobId 42 does not match the task for its taskToken"
                        .to_owned(),
                ),
            ),]
        );
        assert_eq!(
            handler.step_functions.calls(),
            vec![Call::Failure {
                task_token: "token".to_owned(),
                error: Some("ModerationJobMismatch".to_owned()),
                cause: Some(
                    "moderation callback jobId 42 does not match the task for its taskToken"
                        .to_owned(),
                ),
            }]
        );
    }

    #[tokio::test]
    async fn maps_lifecycle_transition_conflicts_to_conflict_responses() {
        let transcription_database = MockDatabase::default();
        transcription_database.set_result(MockDatabaseResult::TransitionConflict);
        let transcription_handler =
            TaskCallbackHandler::new(MockStepFunctions::default(), transcription_database);
        assert_eq!(
            transcription_handler
                .handle(&success_body(
                    "token",
                    &transcription_result("hello".into()),
                ))
                .await,
            409
        );
        assert!(transcription_handler.step_functions.calls().is_empty());

        let moderation_database = MockDatabase::default();
        moderation_database.set_result(MockDatabaseResult::TransitionConflict);
        let moderation_handler =
            TaskCallbackHandler::new(MockStepFunctions::default(), moderation_database);
        assert_eq!(
            moderation_handler
                .handle(&moderation_success_body("token", scores()))
                .await,
            409
        );
        assert!(moderation_handler.step_functions.calls().is_empty());

        let failure_database = MockDatabase::default();
        failure_database.set_failure_result(MockFailureResult::PersistenceTransitionConflict);
        let failure_handler =
            TaskCallbackHandler::new(MockStepFunctions::default(), failure_database);
        assert_eq!(
            failure_handler
                .handle(&failure_body("token", None, None))
                .await,
            409
        );
        assert!(failure_handler.step_functions.calls().is_empty());

        let finalization_database = MockDatabase::default();
        finalization_database.set_failure_result(MockFailureResult::FinalizationTransitionConflict);
        let finalization_handler =
            TaskCallbackHandler::new(MockStepFunctions::default(), finalization_database);
        assert_eq!(
            finalization_handler
                .handle(&failure_body("token", None, None))
                .await,
            409
        );
        assert_eq!(finalization_handler.step_functions.calls().len(), 1);
    }

    #[tokio::test]
    async fn accepts_duplicate_identical_successes() {
        let handler =
            TaskCallbackHandler::new(MockStepFunctions::default(), MockDatabase::default());
        let body = success_body("token-123", &transcription_result("hello world".into()));

        assert_eq!(handler.handle(&body).await, 204);
        assert_eq!(handler.handle(&body).await, 204);
        assert_eq!(handler.database.calls().len(), 2);
        assert_eq!(handler.step_functions.calls().len(), 2);
    }

    #[tokio::test]
    async fn persists_success_before_sending_the_step_functions_callback() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let handler = TaskCallbackHandler::new(
            MockStepFunctions {
                events: Some(Arc::clone(&events)),
                ..Default::default()
            },
            MockDatabase {
                events: Some(Arc::clone(&events)),
                ..Default::default()
            },
        );

        assert_eq!(
            handler
                .handle(&success_body(
                    "token",
                    &transcription_result("hello".into())
                ))
                .await,
            204
        );
        assert_eq!(*events.lock().unwrap(), ["complete-asr", "send-success"]);
    }

    #[tokio::test]
    async fn forwards_failure_and_allows_omitted_error_and_cause() {
        let step_functions = MockStepFunctions::default();
        let handler = TaskCallbackHandler::new(step_functions, MockDatabase::default());

        assert_eq!(
            handler
                .handle(br#"{"taskToken":"token-123","outcome":{"type":"failure","error":"ExternalTaskFailed","cause":"details"}}"#)
                .await,
            204
        );
        assert_eq!(
            handler
                .handle(br#"{"taskToken":"token-456","outcome":{"type":"failure"}}"#)
                .await,
            204
        );
        assert_eq!(
            handler.step_functions.calls(),
            vec![
                Call::Failure {
                    task_token: "token-123".to_owned(),
                    error: Some("ExternalTaskFailed".to_owned()),
                    cause: Some("details".to_owned()),
                },
                Call::Failure {
                    task_token: "token-456".to_owned(),
                    error: None,
                    cause: None,
                },
            ]
        );
    }

    #[tokio::test]
    async fn rejects_malformed_or_missing_input_without_calling_step_functions() {
        let step_functions = MockStepFunctions::default();
        let handler = TaskCallbackHandler::new(step_functions, MockDatabase::default());

        for body in [
            br#"{"taskToken":"token","outcome":"success"}"#.as_slice(),
            br#"{"outcome":{"type":"success","transcriptionResult":{"jobId":"42","asrTaskId":"fc-123","transcription":"hello"}}}"#.as_slice(),
            br#"{"taskToken":"token","outcome":{"type":"success","result":null}}"#.as_slice(),
            br#"{"taskToken":"token","outcome":{"type":"success"}}"#.as_slice(),
            br#"{"taskToken":"token","outcome":{"type":"success","transcriptionResult":{"jobId":"42"}}}"#.as_slice(),
            br#"not json"#.as_slice(),
        ] {
            assert_eq!(handler.handle(body).await, 400);
        }
        assert!(handler.step_functions.calls().is_empty());
    }

    #[tokio::test]
    async fn enforces_all_step_functions_byte_limits_before_calling_the_sdk() {
        let step_functions = MockStepFunctions::default();
        let handler = TaskCallbackHandler::new(step_functions, MockDatabase::default());

        let empty = transcription_result(String::new());
        let output_base = serde_json::to_string(&empty).unwrap().len();

        assert_eq!(handler.handle(&success_body("x", &empty)).await, 204);
        assert_eq!(handler.handle(&success_body("", &empty)).await, 400);
        assert_eq!(
            handler
                .handle(&success_body(&"t".repeat(MAX_TASK_TOKEN_BYTES), &empty))
                .await,
            204
        );
        assert_eq!(
            handler
                .handle(&success_body(&"t".repeat(MAX_TASK_TOKEN_BYTES + 1), &empty))
                .await,
            400
        );
        assert_eq!(
            handler
                .handle(&success_body(
                    "token",
                    &transcription_result("x".repeat(MAX_SUCCESS_OUTPUT_BYTES - output_base)),
                ))
                .await,
            204
        );
        assert_eq!(
            handler
                .handle(&success_body(
                    "token",
                    &transcription_result("x".repeat(MAX_SUCCESS_OUTPUT_BYTES - output_base + 1)),
                ))
                .await,
            400
        );
        assert_eq!(
            handler
                .handle(&failure_body(
                    "token",
                    Some("e".repeat(MAX_ERROR_BYTES)),
                    None
                ))
                .await,
            204
        );
        assert_eq!(
            handler
                .handle(&failure_body(
                    "token",
                    Some("e".repeat(MAX_ERROR_BYTES + 1)),
                    None,
                ))
                .await,
            400
        );
        assert_eq!(
            handler
                .handle(&failure_body(
                    "token",
                    None,
                    Some("c".repeat(MAX_CAUSE_BYTES))
                ))
                .await,
            204
        );
        assert_eq!(
            handler
                .handle(&failure_body(
                    "token",
                    None,
                    Some("c".repeat(MAX_CAUSE_BYTES + 1)),
                ))
                .await,
            400
        );
        assert_eq!(handler.step_functions.calls().len(), 5);
    }

    #[tokio::test]
    async fn maps_step_functions_errors_to_http_statuses() {
        let empty = transcription_result(String::new());
        for (error, expected_status) in [
            (StepFunctionsError::InvalidToken, 409),
            (StepFunctionsError::TaskTimedOut, 409),
            (StepFunctionsError::TaskDoesNotExist, 409),
            (StepFunctionsError::Unavailable, 503),
            (StepFunctionsError::Other, 502),
        ] {
            let step_functions = MockStepFunctions::default();
            step_functions.set_result(Err(error));
            let handler = TaskCallbackHandler::new(step_functions, MockDatabase::default());

            assert_eq!(
                handler.handle(&success_body("token", &empty)).await,
                expected_status
            );
        }
    }

    #[test]
    fn returns_an_empty_no_content_response() {
        let callback_response = response(204);

        assert_eq!(callback_response.status().as_u16(), 204);
        assert!(callback_response.body().as_ref().is_empty());
    }

    #[tokio::test]
    async fn does_not_consume_the_token_when_persistence_fails() {
        let step_functions = MockStepFunctions::default();
        let database = MockDatabase::default();
        database.set_result(MockDatabaseResult::Failure);
        let handler = TaskCallbackHandler::new(step_functions, database);

        assert_eq!(
            handler
                .handle(&success_body(
                    "token",
                    &transcription_result("hello".into())
                ))
                .await,
            500
        );
        assert!(handler.step_functions.calls().is_empty());
        assert_eq!(handler.database.calls().len(), 1);
    }

    #[tokio::test]
    async fn surfaces_a_conflicting_success_without_consuming_the_token() {
        let step_functions = MockStepFunctions::default();
        let database = MockDatabase::default();
        database.set_result(MockDatabaseResult::Conflict);
        let handler = TaskCallbackHandler::new(step_functions, database);

        assert_eq!(
            handler
                .handle(&success_body(
                    "token",
                    &transcription_result("other".into())
                ))
                .await,
            409
        );
        assert!(handler.step_functions.calls().is_empty());
    }

    fn transcription_result(transcription: String) -> TranscriptionResult {
        TranscriptionResult {
            job_id: "42".to_owned(),
            asr_task_id: "fc-123".to_owned(),
            transcription,
        }
    }

    fn success_body(task_token: &str, transcription_result: &TranscriptionResult) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "taskToken": task_token,
            "outcome": { "type": "success", "transcriptionResult": transcription_result },
        }))
        .unwrap()
    }

    fn failure_body(task_token: &str, error: Option<String>, cause: Option<String>) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "taskToken": task_token,
            "outcome": { "type": "failure", "error": error, "cause": cause },
        }))
        .unwrap()
    }

    fn scores() -> HashMap<String, f64> {
        HashMap::from([
            ("sexual".to_owned(), 0.02),
            ("hate_or_discrimination".to_owned(), 0.15),
            ("harassment_or_abuse".to_owned(), 0.08),
            ("violence_or_threats".to_owned(), 0.01),
            ("asking_for_pii".to_owned(), 0.42),
        ])
    }

    fn moderation_success_body(task_token: &str, scores: HashMap<String, f64>) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "taskToken": task_token,
            "outcome": {
                "type": "success",
                "moderationResult": {
                    "jobId": "42",
                    "moderationTaskId": "moderation_0123456789abcdef0123456789abcdef",
                    "scores": scores,
                },
            },
        }))
        .unwrap()
    }
}
