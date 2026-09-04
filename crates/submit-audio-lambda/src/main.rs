use std::env;
use std::io::Error as IoError;

use aws_config::BehaviorVersion;
use aws_sdk_s3::Client as S3Client;
use aws_sdk_ssm::Client as SsmClient;
use lambda_http::{Error, Response, run, service_fn};
use sqlx::postgres::PgPoolOptions;

async fn load_database_url(ssm_client: &SsmClient) -> Result<String, Error> {
    let database_parameter_name =
        env::var("DATABASE_URL_PARAMETER").expect("DATABASE_URL_PARAMETER must be set");
    let database_parameter = ssm_client
        .get_parameter()
        .name(database_parameter_name)
        .with_decryption(true)
        .send()
        .await?;

    database_parameter
        .parameter()
        .and_then(|parameter| parameter.value())
        .map(str::to_owned)
        .ok_or_else(|| IoError::other("database parameter has no value").into())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let sdk_config = aws_config::load_defaults(BehaviorVersion::latest()).await;
    let s3_client = S3Client::new(&sdk_config);
    let ssm_client = SsmClient::new(&sdk_config);

    let database_url = load_database_url(&ssm_client).await?;
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect_lazy(&database_url)?;

    run(service_fn(move |_| {
        let _pool = pool.clone();
        let _s3_client = s3_client.clone();
        async {
            Ok::<_, Error>(
                Response::builder()
                    .status(501)
                    .body("Not implemented".to_string())?,
            )
        }
    }))
    .await
}
