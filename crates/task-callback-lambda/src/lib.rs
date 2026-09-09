use std::future::Future;

use aws_sdk_sfn::Client as SfnClient;
use aws_sdk_sfn::error::{ProvideErrorMetadata, SdkError};
use lambda_http::Body;
use serde::Deserialize;
use serde_json::Value;
use tracing::info;

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
pub struct TaskCallbackHandler<S> {
    step_functions: S,
}

impl<S> TaskCallbackHandler<S> {
    pub fn new(step_functions: S) -> Self {
        Self { step_functions }
    }
}

impl<S: StepFunctions> TaskCallbackHandler<S> {
    pub async fn handle(&self, body: &[u8]) -> u16 {
        let callback = match serde_json::from_slice::<CallbackRequest>(body) {
            Ok(callback) => callback,
            Err(_) => return 400,
        };

        if !(1..=MAX_TASK_TOKEN_BYTES).contains(&callback.task_token.len()) {
            return 400;
        }

        let result = match callback.outcome {
            Outcome::Success { result } => {
                let output = match serde_json::to_string(&result) {
                    Ok(output) if output.len() <= MAX_SUCCESS_OUTPUT_BYTES => output,
                    _ => return 400,
                };
                info!(outcome = "success", "received external task callback");
                self.step_functions
                    .send_task_success(callback.task_token, output)
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
                info!(outcome = "failure", "received external task callback");
                self.step_functions
                    .send_task_failure(callback.task_token, error, cause)
                    .await
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
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CallbackRequest {
    task_token: String,
    outcome: Outcome,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Outcome {
    Success {
        result: Value,
    },
    Failure {
        error: Option<String>,
        cause: Option<String>,
    },
}

pub fn response(status: u16) -> lambda_http::Response<Body> {
    lambda_http::Response::builder()
        .status(status)
        .body(Body::Empty)
        .expect("callback response status is valid")
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    struct MockStepFunctions {
        calls: Mutex<Vec<Call>>,
        result: Mutex<Result<(), StepFunctionsError>>,
    }

    impl Default for MockStepFunctions {
        fn default() -> Self {
            Self {
                calls: Mutex::default(),
                result: Mutex::new(Ok(())),
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
            self.calls.lock().unwrap().push(Call::Failure {
                task_token,
                error,
                cause,
            });
            *self.result.lock().unwrap()
        }
    }

    #[tokio::test]
    async fn forwards_success_result_without_the_task_token() {
        let step_functions = MockStepFunctions::default();
        let handler = TaskCallbackHandler::new(step_functions);
        let body = br#"{"taskToken":"token-123","outcome":{"type":"success","result":{"nested":[null,{"ok":true}],"count":2}}}"#;

        assert_eq!(handler.handle(body).await, 204);
        assert_eq!(
            handler.step_functions.calls(),
            vec![Call::Success {
                task_token: "token-123".to_owned(),
                output: r#"{"count":2,"nested":[null,{"ok":true}]}"#.to_owned(),
            }]
        );
    }

    #[tokio::test]
    async fn forwards_failure_and_allows_omitted_error_and_cause() {
        let step_functions = MockStepFunctions::default();
        let handler = TaskCallbackHandler::new(step_functions);

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
        let handler = TaskCallbackHandler::new(step_functions);

        for body in [
            br#"{"taskToken":"token","outcome":"success"}"#.as_slice(),
            br#"{"outcome":{"type":"success","result":null}}"#.as_slice(),
            br#"{"taskToken":"token","outcome":{"type":"success"}}"#.as_slice(),
            br#"not json"#.as_slice(),
        ] {
            assert_eq!(handler.handle(body).await, 400);
        }
        assert!(handler.step_functions.calls().is_empty());
    }

    #[tokio::test]
    async fn enforces_all_step_functions_byte_limits_before_calling_the_sdk() {
        let step_functions = MockStepFunctions::default();
        let handler = TaskCallbackHandler::new(step_functions);

        assert_eq!(handler.handle(&success_body("x", Value::Null)).await, 204);
        assert_eq!(handler.handle(&success_body("", Value::Null)).await, 400);
        assert_eq!(
            handler
                .handle(&success_body(
                    &"t".repeat(MAX_TASK_TOKEN_BYTES),
                    Value::Null
                ))
                .await,
            204
        );
        assert_eq!(
            handler
                .handle(&success_body(
                    &"t".repeat(MAX_TASK_TOKEN_BYTES + 1),
                    Value::Null
                ))
                .await,
            400
        );
        assert_eq!(
            handler
                .handle(&success_body(
                    "token",
                    Value::String("x".repeat(MAX_SUCCESS_OUTPUT_BYTES - 2)),
                ))
                .await,
            204
        );
        assert_eq!(
            handler
                .handle(&success_body(
                    "token",
                    Value::String("x".repeat(MAX_SUCCESS_OUTPUT_BYTES - 1)),
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
        for (error, expected_status) in [
            (StepFunctionsError::InvalidToken, 409),
            (StepFunctionsError::TaskTimedOut, 409),
            (StepFunctionsError::TaskDoesNotExist, 409),
            (StepFunctionsError::Unavailable, 503),
            (StepFunctionsError::Other, 502),
        ] {
            let step_functions = MockStepFunctions::default();
            step_functions.set_result(Err(error));
            let handler = TaskCallbackHandler::new(step_functions);

            assert_eq!(
                handler.handle(&success_body("token", Value::Null)).await,
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

    fn success_body(task_token: &str, result: Value) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "taskToken": task_token,
            "outcome": { "type": "success", "result": result },
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
}
