use audio_processing_lambda::{AudioProcessingHandler, AudioProcessingInput, AwsS3Storage};
use aws_config::BehaviorVersion;
use aws_sdk_s3::Client as S3Client;
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

    let sdk_config = aws_config::load_defaults(BehaviorVersion::latest()).await;
    let handler = AudioProcessingHandler::new(AwsS3Storage::new(S3Client::new(&sdk_config)));

    run(service_fn(
        move |event: LambdaEvent<AudioProcessingInput>| {
            let handler = handler.clone();
            async move { handler.handle(event.payload).await }
        },
    ))
    .await
}
