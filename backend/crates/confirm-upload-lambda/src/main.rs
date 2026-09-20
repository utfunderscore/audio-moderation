use std::env;

use aws_config::BehaviorVersion;
use aws_sdk_s3::Client as S3Client;
use aws_sdk_sfn::Client as SfnClient;
use aws_sdk_ssm::Client as SsmClient;
use common::{evaluation_dispatch::EvaluationDispatcher, load_database_url};
use confirm_upload_lambda::ConfirmUploadHandler;
use database::{
    PipelineTaskEventStore, PipelineTaskStore, PipelineTaskWebSocketConnectionStore, ReviewJobStore,
};
use lambda_runtime::{Error, run, service_fn};
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
    let database_url = load_database_url(&ssm_client).await?;
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect_lazy(&database_url)?;
    let uploads_bucket = env::var("UPLOADS_BUCKET_NAME").expect("UPLOADS_BUCKET_NAME must be set");
    let artifacts_bucket =
        env::var("ARTIFACTS_BUCKET_NAME").expect("ARTIFACTS_BUCKET_NAME must be set");
    let state_machine_arn = env::var("STATE_MACHINE_ARN").expect("STATE_MACHINE_ARN must be set");
    let tenant_id = env::var("TENANT_ID").expect("TENANT_ID must be set");
    let events = TaskEventEmitter::new(
        PipelineTaskEventStore::new(pool.clone()),
        PipelineTaskWebSocketConnectionStore::new(pool.clone()),
        &sdk_config,
        env::var("TASK_EVENTS_MANAGEMENT_ENDPOINT")
            .expect("TASK_EVENTS_MANAGEMENT_ENDPOINT must be set"),
    );
    let dispatcher = EvaluationDispatcher::new(
        PipelineTaskStore::new(pool.clone()),
        SfnClient::new(&sdk_config),
        state_machine_arn,
        artifacts_bucket,
        events,
    );
    let handler = ConfirmUploadHandler::new(
        ReviewJobStore::new(pool.clone()),
        PipelineTaskStore::new(pool),
        dispatcher,
        S3Client::new(&sdk_config),
        uploads_bucket,
        tenant_id,
    );

    run(service_fn(move |event| {
        let handler = handler.clone();
        async move { handler.handle(event).await }
    }))
    .await
}
