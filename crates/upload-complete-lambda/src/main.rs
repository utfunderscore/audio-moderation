use std::env;
use std::io::Error as IoError;

use aws_config::BehaviorVersion;
use aws_sdk_s3::Client as S3Client;
use aws_sdk_ssm::Client as SsmClient;
use database::ReviewJobStore;
use lambda_runtime::{Error, run, service_fn};
use sqlx::postgres::PgPoolOptions;
use upload_complete_lambda::UploadCompleteHandler;

async fn load_database_url(ssm_client: &SsmClient) -> Result<String, Error> {
    let parameter_name =
        env::var("DATABASE_URL_PARAMETER").expect("DATABASE_URL_PARAMETER must be set");
    let response = ssm_client
        .get_parameter()
        .name(parameter_name)
        .with_decryption(true)
        .send()
        .await?;

    response
        .parameter()
        .and_then(|parameter| parameter.value())
        .map(str::to_owned)
        .ok_or_else(|| IoError::other("database parameter has no value").into())
}

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
    let tenant_id = env::var("TENANT_ID").expect("TENANT_ID must be set");
    let handler = UploadCompleteHandler::new(
        ReviewJobStore::new(pool),
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
