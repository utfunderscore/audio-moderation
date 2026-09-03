use std::env;

use aws_config::BehaviorVersion;
use aws_sdk_s3::Client as S3Client;
use lambda_http::{Error, Response, run, service_fn};
use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let sdk_config = aws_config::load_defaults(BehaviorVersion::latest()).await;
    let s3_client = S3Client::new(&sdk_config);

    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect_lazy(&database_url)?;

    run(service_fn(move |_| {
        let _pool = pool.clone();
        let _s3_client = s3_client.clone();
        async {
            Ok::<_, Error>(Response::builder()
                .status(501)
                .body("Not implemented".to_string())?)
        }
    }))
    .await
}
