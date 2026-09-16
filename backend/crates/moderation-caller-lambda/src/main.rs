use std::{env, time::Duration};

use aws_config::BehaviorVersion;
use aws_sdk_ssm::Client as SsmClient;
use common::{load_database_url, load_secure_parameter};
use database::{PipelineTaskEventStore, PipelineTaskStore, PipelineTaskWebSocketConnectionStore};
use lambda_runtime::{Error, LambdaEvent, run, service_fn};
use moderation_caller_lambda::{
    ModalModerationClient, ModerationCallerHandler, ModerationCallerInput,
};
use reqwest::Client as HttpClient;
use sqlx::postgres::PgPoolOptions;
use task_event_emitter::TaskEventEmitter;

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
    let proxy_key = load_secure_parameter(&ssm_client, "MODAL_PROXY_TOKEN_ID_PARAMETER").await?;
    let proxy_secret =
        load_secure_parameter(&ssm_client, "MODAL_PROXY_TOKEN_SECRET_PARAMETER").await?;
    let database_url = load_database_url(&ssm_client).await?;
    let modal_endpoint = env::var("MODAL_ENDPOINT_URL").expect("MODAL_ENDPOINT_URL must be set");
    let http_client = HttpClient::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect_lazy(&database_url)?;
    let database = PipelineTaskStore::new(pool.clone());
    let events = TaskEventEmitter::new(
        PipelineTaskEventStore::new(pool.clone()),
        PipelineTaskWebSocketConnectionStore::new(pool),
        &sdk_config,
        env::var("TASK_EVENTS_MANAGEMENT_ENDPOINT")
            .expect("TASK_EVENTS_MANAGEMENT_ENDPOINT must be set"),
    );
    let handler = ModerationCallerHandler::new(
        ModalModerationClient::new(http_client, modal_endpoint, proxy_key, proxy_secret),
        database,
    )
    .with_event_emitter(events);

    run(service_fn(
        move |event: LambdaEvent<ModerationCallerInput>| {
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
