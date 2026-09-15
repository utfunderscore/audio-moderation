use aws_config::BehaviorVersion;
use aws_sdk_sfn::Client as SfnClient;
use aws_sdk_ssm::Client as SsmClient;
use common::load_database_url;
use database::PipelineTaskStore;
use lambda_http::{Error, Request, run, service_fn};
use sqlx::postgres::PgPoolOptions;
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
    let database_url = load_database_url(&SsmClient::new(&sdk_config)).await?;
    let database = PipelineTaskStore::new(
        PgPoolOptions::new()
            .max_connections(3)
            .connect_lazy(&database_url)?,
    );
    let handler =
        TaskCallbackHandler::new(AwsStepFunctions::new(SfnClient::new(&sdk_config)), database);

    run(service_fn(move |request: Request| {
        let handler = handler.clone();
        async move { Ok::<_, Error>(response(handler.handle(request.body().as_ref()).await)) }
    }))
    .await
}
