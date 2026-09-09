use aws_config::BehaviorVersion;
use aws_sdk_sfn::Client as SfnClient;
use lambda_http::{Error, Request, run, service_fn};
use task_callback_lambda::{AwsStepFunctions, TaskCallbackHandler, response};

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .json()
        .without_time()
        .init();

    let sdk_config = aws_config::load_defaults(BehaviorVersion::latest()).await;
    let handler = TaskCallbackHandler::new(AwsStepFunctions::new(SfnClient::new(&sdk_config)));

    run(service_fn(move |request: Request| {
        let handler = handler.clone();
        async move { Ok::<_, Error>(response(handler.handle(request.body().as_ref()).await)) }
    }))
    .await
}
