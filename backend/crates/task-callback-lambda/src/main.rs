use aws_config::BehaviorVersion;
use aws_sdk_sfn::Client as SfnClient;
use aws_sdk_ssm::Client as SsmClient;
use common::load_database_url;
use database::{PipelineTaskEventStore, PipelineTaskStore, PipelineTaskWebSocketConnectionStore};
use lambda_http::{Error, Request, run, service_fn};
use sqlx::postgres::PgPoolOptions;
use task_callback_lambda::{AwsStepFunctions, TaskCallbackHandler, response};
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
    let database_url = load_database_url(&SsmClient::new(&sdk_config)).await?;
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
    let handler =
        TaskCallbackHandler::new(AwsStepFunctions::new(SfnClient::new(&sdk_config)), database)
            .with_event_emitter(events);

    run(service_fn(move |request: Request| {
        let handler = handler.clone();
        async move { Ok::<_, Error>(response(handler.handle(request.body().as_ref()).await)) }
    }))
    .await
}
use std::env;
