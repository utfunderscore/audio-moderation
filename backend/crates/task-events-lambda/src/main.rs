use aws_config::BehaviorVersion;
use aws_sdk_ssm::Client as SsmClient;
use common::load_database_url;
use database::{
    PipelineTaskEventStore, PipelineTaskEventTicketStore, PipelineTaskWebSocketConnectionStore,
};
use lambda_runtime::{Error, run, service_fn};
use sqlx::postgres::PgPoolOptions;
use task_event_emitter::TaskEventEmitter;
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
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect_lazy(&database_url)?;
    let connections = PipelineTaskWebSocketConnectionStore::new(pool.clone());
    let tickets = PipelineTaskEventTicketStore::new(pool.clone());
    let events = TaskEventEmitter::new(
        PipelineTaskEventStore::new(pool),
        connections.clone(),
        &sdk_config,
        env::var("TASK_EVENTS_MANAGEMENT_ENDPOINT")
            .expect("TASK_EVENTS_MANAGEMENT_ENDPOINT must be set"),
    );
    let handler = TaskEventsHandler::new(connections, tickets).with_event_emitter(events);
    run(service_fn(move |event| {
        let handler = handler.clone();
        async move { handler.handle(event).await }
    }))
    .await
}
use std::env;
