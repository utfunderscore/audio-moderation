use sqlx::{PgPool, postgres::PgPoolOptions};
use testcontainers::{
    ContainerAsync, GenericImage, ImageExt,
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
};

pub struct TestDatabase {
    _container: ContainerAsync<GenericImage>,
    pub pool: PgPool,
}

impl TestDatabase {
    pub async fn start() -> Self {
        let container = GenericImage::new("postgres", "17-alpine")
            .with_exposed_port(5432.tcp())
            .with_wait_for(WaitFor::message_on_stderr(
                "database system is ready to accept connections",
            ))
            .with_env_var("POSTGRES_PASSWORD", "postgres")
            .with_env_var("POSTGRES_DB", "audio_moderation")
            .start()
            .await
            .expect("PostgreSQL container should start");
        let host = container
            .get_host()
            .await
            .expect("container host should be available");
        let port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("PostgreSQL port should be available");
        let database_url = format!("postgres://postgres:postgres@{host}:{port}/audio_moderation");
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .expect("test database should accept connections");

        sqlx::raw_sql(include_str!(
            "../../../../migrations/V1__create_review_jobs.sql"
        ))
        .execute(&pool)
        .await
        .expect("review job migration should apply");
        sqlx::raw_sql(include_str!(
            "../../../../migrations/V2__create_pipeline_tasks.sql"
        ))
        .execute(&pool)
        .await
        .expect("pipeline task migration should apply");

        Self {
            _container: container,
            pool,
        }
    }
}
