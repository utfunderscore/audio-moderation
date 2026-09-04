use audio_processing_lambda::{AudioProcessingInput, handle};
use lambda_runtime::{Error, LambdaEvent, run, service_fn};

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .json()
        .without_time()
        .init();

    run(service_fn(
        |event: LambdaEvent<AudioProcessingInput>| async move { handle(event.payload).await },
    ))
    .await
}
