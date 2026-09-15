use audio_processing_lambda::{AudioProcessingHandler, AudioWorkerInput, AwsS3Storage};
use aws_config::BehaviorVersion;
use aws_sdk_s3::Client as S3Client;
use aws_sdk_ssm::Client as SsmClient;
use common::load_database_url;
use database::PipelineTaskStore;
use lambda_runtime::{Error, LambdaEvent, run, service_fn};
use sqlx::postgres::PgPoolOptions;

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
    let database_url = load_database_url(&SsmClient::new(&sdk_config)).await?;
    let database = PipelineTaskStore::new(
        PgPoolOptions::new()
            .max_connections(3)
            .connect_lazy(&database_url)?,
    );
    let handler = AudioProcessingHandler::new(
        AwsS3Storage::new(S3Client::new(&sdk_config)),
        database.clone(),
    );

    run(service_fn(move |event: LambdaEvent<AudioWorkerInput>| {
        let handler = handler.clone();
        let database = database.clone();
        async move {
            match event.payload {
                AudioWorkerInput::Audio(input) => handler
                    .handle(input)
                    .await
                    .and_then(|output| serde_json::to_value(output).map_err(Into::into)),
                AudioWorkerInput::Lifecycle(input) => {
                    let task_id = input.job_id.parse()?;
                    database
                        .finish_workflow(
                            task_id,
                            input.outcome.into(),
                            None,
                            input.error.as_deref(),
                            input.cause.as_deref(),
                        )
                        .await?;
                    Ok(serde_json::Value::Null)
                }
                AudioWorkerInput::ExecutionStatus(event) => {
                    let input = event.lifecycle()?;
                    let task_id = input.job_id.parse()?;
                    database
                        .finish_workflow(
                            task_id,
                            input.outcome.into(),
                            None,
                            input.error.as_deref(),
                            input.cause.as_deref(),
                        )
                        .await?;
                    Ok(serde_json::Value::Null)
                }
            }
        }
    }))
    .await
}
