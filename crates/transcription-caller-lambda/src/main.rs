use std::{env, time::Duration};

use aws_config::BehaviorVersion;
use aws_sdk_ssm::Client as SsmClient;
use common::{load_database_url, load_secure_parameter};
use database::PipelineTaskStore;
use lambda_runtime::{Error, LambdaEvent, run, service_fn};
use reqwest::Client as HttpClient;
use sqlx::postgres::PgPoolOptions;
use transcription_caller_lambda::{
    ModalAsrClient, TranscriptionCallerHandler, TranscriptionCallerInput,
};

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
    let ssm_client = SsmClient::new(&sdk_config);
    let token_id = load_secure_parameter(&ssm_client, "MODAL_PROXY_TOKEN_ID_PARAMETER").await?;
    let token_secret =
        load_secure_parameter(&ssm_client, "MODAL_PROXY_TOKEN_SECRET_PARAMETER").await?;
    let database_url = load_database_url(&ssm_client).await?;
    let endpoint =
        env::var("TRANSCRIPTION_ENDPOINT_URL").expect("TRANSCRIPTION_ENDPOINT_URL must be set");
    let http_client = HttpClient::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    let database = PipelineTaskStore::new(
        PgPoolOptions::new()
            .max_connections(3)
            .connect_lazy(&database_url)?,
    );
    let handler = TranscriptionCallerHandler::new(
        ModalAsrClient::new(http_client, endpoint, format!("{token_id}.{token_secret}")),
        database,
    );

    run(service_fn(
        move |event: LambdaEvent<TranscriptionCallerInput>| {
            let handler = handler.clone();
            async move {
                handler
                    .handle(event.payload)
                    .await
                    .map_err(|error| -> Error { Box::new(error) })
            }
        },
    ))
    .await
}
