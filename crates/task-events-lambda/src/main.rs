use aws_config::BehaviorVersion;
use aws_sdk_ssm::Client as SsmClient;
use common::load_database_url;
use database::PipelineTaskWebSocketConnectionStore;
use lambda_runtime::{Error, run, service_fn};
use sqlx::postgres::PgPoolOptions;
use task_events_lambda::TaskEventsHandler;

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
    let connections = PipelineTaskWebSocketConnectionStore::new(
        PgPoolOptions::new()
            .max_connections(3)
            .connect_lazy(&database_url)?,
    );
    let handler = TaskEventsHandler::new(connections);
    run(service_fn(move |event| {
        let handler = handler.clone();
        async move { handler.handle(event).await }
    }))
    .await
}
