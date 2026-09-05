use std::env;

use aws_config::BehaviorVersion;
use aws_sdk_s3::Client as S3Client;
use aws_sdk_ssm::Client as SsmClient;
use common::load_database_url;
use connectrpc::ConnectRpcService;
use database::ReviewJobStore;
use http_body_util::Full;
use lambda_http::{Error, Request as LambdaRequest, run, service_fn};
use sqlx::postgres::PgPoolOptions;
use tower::Service;

mod proto;
mod service;

use proto::audio::review::v1::AudioReviewServiceServer;
use service::SubmitReviewService;

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
    let s3_client = S3Client::new(&sdk_config);
    let ssm_client = SsmClient::new(&sdk_config);

    let database_url = load_database_url(&ssm_client).await?;
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect_lazy(&database_url)?;
    let uploads_bucket = env::var("UPLOADS_BUCKET_NAME").expect("UPLOADS_BUCKET_NAME must be set");
    let tenant_id = env::var("TENANT_ID").expect("TENANT_ID must be set");

    let service = SubmitReviewService::new(
        ReviewJobStore::new(pool),
        s3_client,
        uploads_bucket,
        tenant_id,
    );
    let connect_service = ConnectRpcService::new(AudioReviewServiceServer::new(service));

    run(service_fn(move |request: LambdaRequest| {
        let mut connect_service = connect_service.clone();
        async move {
            let request = request.map(|body| Full::new(body.as_ref().to_owned().into()));
            connect_service
                .call(request)
                .await
                .map_err(|never| -> Error { match never {} })
        }
    }))
    .await
}
