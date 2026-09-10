use std::env;
use std::error::Error;
use std::io::{Error as IoError, ErrorKind};
use std::time::Duration;

use aws_config::BehaviorVersion;
use aws_sdk_sfn::Client as SfnClient;
use database::{NewPipelineTask, PipelineTask, PipelineTaskStore};
use reqwest::StatusCode;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

const START_EVALUATION_PATH: &str = "/audio.moderation.v1.AudioModerationService/StartEvaluation";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

struct Environment {
    endpoint: String,
    tenant_id: String,
    audio_s3_uris: Vec<String>,
    pool: PgPool,
}

struct StoredTask {
    task_id: i32,
    caller_reference: Option<String>,
    status: String,
    execution_arn: Option<String>,
    attempt_count: i32,
    active_lease: bool,
}

struct HttpResponse {
    status: StatusCode,
    body: String,
}

#[tokio::test]
#[ignore = "requires a deployed start-evaluation Lambda, PostgreSQL, and pre-uploaded S3 audio"]
async fn rejects_invalid_requests_without_writing_tasks() -> Result<(), Box<dyn Error>> {
    let environment = Environment::load().await?;
    let client = http_client()?;
    let prefix = format!("deployed-validation-{}", Uuid::new_v4());
    let url = environment.start_url();

    let missing_key = post_start(
        &client,
        &url,
        None,
        &environment.audio_s3_uris,
        &format!("{prefix}-missing-key"),
    )
    .await?;
    assert_connect_error(&missing_key, StatusCode::BAD_REQUEST, "invalid_argument");

    let blank_key = post_start(
        &client,
        &url,
        Some("   "),
        &environment.audio_s3_uris,
        &format!("{prefix}-blank-key"),
    )
    .await?;
    assert_connect_error(&blank_key, StatusCode::BAD_REQUEST, "invalid_argument");

    let empty_objects = post_start(
        &client,
        &url,
        Some(&format!("{prefix}-empty-objects")),
        &[],
        &format!("{prefix}-empty-objects"),
    )
    .await?;
    assert_connect_error(&empty_objects, StatusCode::BAD_REQUEST, "invalid_argument");

    for (name, uri) in [
        ("not-s3", "https://example.invalid/audio.wav"),
        ("missing-key", "s3://upload-bucket"),
        ("missing-bucket", "s3:///audio.wav"),
    ] {
        let response = post_start(
            &client,
            &url,
            Some(&format!("{prefix}-{name}")),
            &[uri.to_owned()],
            &format!("{prefix}-{name}"),
        )
        .await?;
        assert_connect_error(&response, StatusCode::BAD_REQUEST, "invalid_argument");
    }

    // The store intentionally has no read accessor; this is a read-only,
    // dynamic SQL assertion so the test never creates a task while verifying it.
    let created_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pipeline_tasks WHERE tenant_id = $1 AND caller_reference LIKE $2",
    )
    .bind(&environment.tenant_id)
    .bind(format!("{prefix}%"))
    .fetch_one(&environment.pool)
    .await?;
    assert_eq!(created_count, 0, "invalid requests must not create tasks");

    Ok(())
}

#[tokio::test]
#[ignore = "requires a deployed start-evaluation Lambda, PostgreSQL, and pre-uploaded S3 audio"]
async fn completes_a_fresh_evaluation_and_replays_without_another_attempt()
-> Result<(), Box<dyn Error>> {
    let environment = Environment::load().await?;
    let client = http_client()?;
    let key = unique_key("fresh");
    let caller_reference = unique_key("caller");

    let created = successful_json(
        post_start(
            &client,
            &environment.start_url(),
            Some(&key),
            &environment.audio_s3_uris,
            &caller_reference,
        )
        .await?,
    )?;
    let task = stored_task(&environment.pool, &environment.tenant_id, &key).await?;
    assert_eq!(created["evaluationId"], task.task_id.to_string());
    assert_eq!(created["status"], "PIPELINE_TASK_STATUS_PENDING");
    assert_eq!(
        task.caller_reference.as_deref(),
        Some(caller_reference.as_str())
    );
    assert_eq!(task.attempt_count, 1);
    assert!(!task.active_lease);
    assert_eq!(
        stored_inputs(&environment.pool, task.task_id).await?,
        environment.audio_s3_uris
    );
    assert_dispatched(&task, &environment.audio_s3_uris).await?;
    let execution_arn = task
        .execution_arn
        .as_deref()
        .expect("dispatched task must have an execution ARN");
    wait_for_execution_success(execution_arn).await?;

    let store = PipelineTaskStore::new(environment.pool.clone());
    let completed = seed_task(&store, &environment, &key, &caller_reference).await?;
    assert!(
        completed
            .asr_task_id
            .as_deref()
            .is_some_and(|id| !id.is_empty()),
        "completed workflow must persist an ASR task ID"
    );

    let replayed = successful_json(
        post_start(
            &client,
            &environment.start_url(),
            Some(&key),
            &environment.audio_s3_uris,
            &caller_reference,
        )
        .await?,
    )?;
    let after_replay = stored_task(&environment.pool, &environment.tenant_id, &key).await?;
    assert_eq!(replayed["evaluationId"], task.task_id.to_string());
    assert_eq!(after_replay.task_id, task.task_id);
    assert_eq!(
        after_replay.attempt_count, 1,
        "replay must not claim another dispatch"
    );
    assert_eq!(after_replay.execution_arn, task.execution_arn);

    Ok(())
}

#[tokio::test]
#[ignore = "requires a deployed start-evaluation Lambda, PostgreSQL, and pre-uploaded S3 audio"]
async fn dispatches_a_seeded_undispatched_task() -> Result<(), Box<dyn Error>> {
    let environment = Environment::load().await?;
    let client = http_client()?;
    let key = unique_key("seeded-undispatched");
    let caller_reference = unique_key("caller");
    let store = PipelineTaskStore::new(environment.pool.clone());
    let seeded = seed_task(&store, &environment, &key, &caller_reference).await?;

    let response = successful_json(
        post_start(
            &client,
            &environment.start_url(),
            Some(&key),
            &environment.audio_s3_uris,
            &caller_reference,
        )
        .await?,
    )?;
    let task = stored_task(&environment.pool, &environment.tenant_id, &key).await?;
    assert_eq!(response["evaluationId"], seeded.task_id.to_string());
    assert_eq!(task.attempt_count, 1);
    assert_dispatched(&task, &environment.audio_s3_uris).await?;

    Ok(())
}

#[tokio::test]
#[ignore = "requires a deployed start-evaluation Lambda, PostgreSQL, and pre-uploaded S3 audio"]
async fn retries_a_seeded_failed_dispatch() -> Result<(), Box<dyn Error>> {
    let environment = Environment::load().await?;
    let client = http_client()?;
    let key = unique_key("retry");
    let caller_reference = unique_key("caller");
    let store = PipelineTaskStore::new(environment.pool.clone());
    let seeded = seed_task(&store, &environment, &key, &caller_reference).await?;
    assert_eq!(store.claim_dispatch(seeded.task_id).await?, Some(1));
    store
        .record_dispatch_failure(seeded.task_id, "seeded transient dispatch failure")
        .await?;

    successful_json(
        post_start(
            &client,
            &environment.start_url(),
            Some(&key),
            &environment.audio_s3_uris,
            &caller_reference,
        )
        .await?,
    )?;
    let task = stored_task(&environment.pool, &environment.tenant_id, &key).await?;
    assert_eq!(task.attempt_count, 2);
    assert_dispatched(&task, &environment.audio_s3_uris).await?;

    Ok(())
}

#[tokio::test]
#[ignore = "requires a deployed start-evaluation Lambda, PostgreSQL, and pre-uploaded S3 audio"]
async fn skips_active_leases_and_terminal_failed_tasks() -> Result<(), Box<dyn Error>> {
    let environment = Environment::load().await?;
    let client = http_client()?;
    let store = PipelineTaskStore::new(environment.pool.clone());

    let active_key = unique_key("active-lease");
    let active_caller = unique_key("caller");
    let active = seed_task(&store, &environment, &active_key, &active_caller).await?;
    assert_eq!(store.claim_dispatch(active.task_id).await?, Some(1));
    successful_json(
        post_start(
            &client,
            &environment.start_url(),
            Some(&active_key),
            &environment.audio_s3_uris,
            &active_caller,
        )
        .await?,
    )?;
    let active_after = stored_task(&environment.pool, &environment.tenant_id, &active_key).await?;
    assert_eq!(active_after.attempt_count, 1);
    assert!(active_after.active_lease);
    assert!(active_after.execution_arn.is_none());

    let failed_key = unique_key("terminal-failed");
    let failed_caller = unique_key("caller");
    let failed = seed_task(&store, &environment, &failed_key, &failed_caller).await?;
    for attempt in 1..=3 {
        assert_eq!(store.claim_dispatch(failed.task_id).await?, Some(attempt));
        store
            .record_dispatch_failure(failed.task_id, "seeded dispatch failure")
            .await?;
    }
    let response = successful_json(
        post_start(
            &client,
            &environment.start_url(),
            Some(&failed_key),
            &environment.audio_s3_uris,
            &failed_caller,
        )
        .await?,
    )?;
    let failed_after = stored_task(&environment.pool, &environment.tenant_id, &failed_key).await?;
    assert_eq!(response["status"], "PIPELINE_TASK_STATUS_FAILED");
    assert_eq!(failed_after.status, "FAILED");
    assert_eq!(failed_after.attempt_count, 3);
    assert!(!failed_after.active_lease);
    assert!(failed_after.execution_arn.is_none());

    Ok(())
}

#[tokio::test]
#[ignore = "requires a deployed start-evaluation Lambda, PostgreSQL, and pre-uploaded S3 audio"]
async fn rejects_conflicting_payloads_for_a_seeded_leased_task() -> Result<(), Box<dyn Error>> {
    let environment = Environment::load().await?;
    let client = http_client()?;
    let key = unique_key("payload-conflict");
    let caller_reference = unique_key("caller");
    let store = PipelineTaskStore::new(environment.pool.clone());
    let seeded = seed_task(&store, &environment, &key, &caller_reference).await?;
    assert_eq!(store.claim_dispatch(seeded.task_id).await?, Some(1));

    let mut reordered = environment.audio_s3_uris.clone();
    reordered.swap(0, 1);
    let reordered_response = post_start(
        &client,
        &environment.start_url(),
        Some(&key),
        &reordered,
        &caller_reference,
    )
    .await?;
    assert_connect_error(&reordered_response, StatusCode::CONFLICT, "already_exists");

    let changed_reference_response = post_start(
        &client,
        &environment.start_url(),
        Some(&key),
        &environment.audio_s3_uris,
        &unique_key("different-caller"),
    )
    .await?;
    assert_connect_error(
        &changed_reference_response,
        StatusCode::CONFLICT,
        "already_exists",
    );

    let after_conflicts = stored_task(&environment.pool, &environment.tenant_id, &key).await?;
    assert_eq!(after_conflicts.task_id, seeded.task_id);
    assert_eq!(after_conflicts.attempt_count, 1);
    assert!(after_conflicts.active_lease);
    assert!(after_conflicts.execution_arn.is_none());

    Ok(())
}

impl Environment {
    async fn load() -> Result<Self, Box<dyn Error>> {
        let endpoint = required_env("AUDIO_MODERATION_API_ENDPOINT")?;
        let tenant_id = required_env("AUDIO_MODERATION_TENANT_ID")?;
        let database_url = required_env("DATABASE_URL")?;
        let audio_s3_uris: Vec<String> =
            serde_json::from_str(&required_env("AUDIO_MODERATION_TEST_AUDIO_S3_URIS")?)?;
        if audio_s3_uris.len() < 2 {
            return Err(invalid_input(
                "AUDIO_MODERATION_TEST_AUDIO_S3_URIS must contain at least two audio object URIs",
            ));
        }
        if audio_s3_uris[0] == audio_s3_uris[1] {
            return Err(invalid_input(
                "the first two audio object URIs must differ to test ordering conflicts",
            ));
        }
        if audio_s3_uris.iter().any(|uri| !is_s3_object_uri(uri)) {
            return Err(invalid_input(
                "AUDIO_MODERATION_TEST_AUDIO_S3_URIS must contain s3://bucket/key URIs",
            ));
        }

        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(&database_url)
            .await?;
        Ok(Self {
            endpoint,
            tenant_id,
            audio_s3_uris,
            pool,
        })
    }

    fn start_url(&self) -> String {
        format!(
            "{}{}",
            self.endpoint.trim_end_matches('/'),
            START_EVALUATION_PATH
        )
    }
}

fn required_env(name: &str) -> Result<String, Box<dyn Error>> {
    let value = env::var(name)?;
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err(invalid_input(&format!("{name} must not be blank")));
    }
    Ok(value)
}

fn invalid_input(message: &str) -> Box<dyn Error> {
    IoError::new(ErrorKind::InvalidInput, message.to_owned()).into()
}

fn is_s3_object_uri(uri: &str) -> bool {
    uri.strip_prefix("s3://").is_some_and(|location| {
        location.contains('/') && !location.starts_with('/') && !location.ends_with('/')
    })
}

fn http_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build()
}

fn unique_key(kind: &str) -> String {
    format!("deployed-{kind}-{}", Uuid::new_v4())
}

async fn post_start(
    client: &reqwest::Client,
    url: &str,
    idempotency_key: Option<&str>,
    audio_s3_uris: &[String],
    caller_reference: &str,
) -> Result<HttpResponse, reqwest::Error> {
    let audio_objects: Vec<_> = audio_s3_uris
        .iter()
        .map(|s3_uri| json!({ "s3Uri": s3_uri }))
        .collect();
    let mut request = client
        .post(url)
        .header("content-type", "application/json")
        .header("connect-protocol-version", "1")
        .json(&json!({
            "audioObjects": audio_objects,
            "callerReference": caller_reference,
        }));
    if let Some(idempotency_key) = idempotency_key {
        request = request.header("idempotency-key", idempotency_key);
    }

    let response = request.send().await?;
    let status = response.status();
    let body = response.text().await?;
    Ok(HttpResponse { status, body })
}

fn assert_connect_error(response: &HttpResponse, status: StatusCode, code: &str) {
    assert_eq!(
        response.status, status,
        "unexpected HTTP response: {}",
        response.body
    );
    let body: Value =
        serde_json::from_str(&response.body).expect("Connect error response must contain JSON");
    assert_eq!(body["code"], code, "unexpected Connect error: {body}");
}

fn successful_json(response: HttpResponse) -> Result<Value, Box<dyn Error>> {
    assert!(
        response.status.is_success(),
        "start evaluation failed with {}: {}",
        response.status,
        response.body
    );
    Ok(serde_json::from_str(&response.body)?)
}

async fn seed_task(
    store: &PipelineTaskStore,
    environment: &Environment,
    key: &str,
    caller_reference: &str,
) -> Result<PipelineTask, Box<dyn Error>> {
    Ok(store
        .create_or_get(NewPipelineTask {
            tenant_id: &environment.tenant_id,
            idempotency_key: key,
            caller_reference: Some(caller_reference),
            audio_s3_uris: &environment.audio_s3_uris,
        })
        .await?)
}

async fn stored_task(
    pool: &PgPool,
    tenant_id: &str,
    idempotency_key: &str,
) -> Result<StoredTask, sqlx::Error> {
    let row: (i32, Option<String>, String, Option<String>, i32, bool) = sqlx::query_as(
        r#"
            SELECT task_id, caller_reference, status::TEXT, execution_arn, attempt_count,
                dispatch_started_at IS NOT NULL AS active_lease
            FROM pipeline_tasks
            WHERE tenant_id = $1 AND idempotency_key = $2
        "#,
    )
    .bind(tenant_id)
    .bind(idempotency_key)
    .fetch_one(pool)
    .await?;
    Ok(StoredTask {
        task_id: row.0,
        caller_reference: row.1,
        status: row.2,
        execution_arn: row.3,
        attempt_count: row.4,
        active_lease: row.5,
    })
}

async fn stored_inputs(pool: &PgPool, task_id: i32) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT audio_s3_uri FROM pipeline_task_inputs WHERE task_id = $1 ORDER BY sequence",
    )
    .bind(task_id)
    .fetch_all(pool)
    .await
}

async fn assert_dispatched(
    task: &StoredTask,
    expected_uris: &[String],
) -> Result<(), Box<dyn Error>> {
    let execution_arn = task
        .execution_arn
        .as_deref()
        .expect("dispatched task must persist its Step Functions execution ARN");
    assert!(execution_arn.starts_with("arn:aws:states:"));
    assert!(execution_arn.contains(":execution:"));
    assert!(
        execution_arn.ends_with(&format!(":evaluation-{}", task.task_id)),
        "unexpected execution ARN: {execution_arn}"
    );

    // Do not inherit an arbitrary local profile: deployment assertions always
    // describe the execution through the repository's admin AWS profile.
    let sdk_config = aws_config::defaults(BehaviorVersion::latest())
        .profile_name("admin")
        .load()
        .await;
    let execution = SfnClient::new(&sdk_config)
        .describe_execution()
        .execution_arn(execution_arn)
        .send()
        .await?;
    let input: Value = serde_json::from_str(
        execution
            .input()
            .expect("Step Functions execution must retain its input"),
    )?;

    assert_eq!(input["jobId"], task.task_id.to_string());
    let files = input["files"]
        .as_array()
        .expect("execution input files must be an array");
    assert_eq!(files.len(), expected_uris.len());
    for (sequence, expected_uri) in expected_uris.iter().enumerate() {
        assert_eq!(files[sequence]["sequence"], sequence);
        assert_eq!(files[sequence]["s3Uri"], expected_uri.as_str());
    }
    let output_s3_uri = input["outputS3Uri"]
        .as_str()
        .expect("execution input must have outputS3Uri");
    let expected_path = format!("evaluations/{}.wav", task.task_id);
    assert!(
        output_s3_uri
            .strip_prefix("s3://")
            .and_then(|location| location.split_once('/'))
            .is_some_and(|(bucket, path)| !bucket.is_empty() && path == expected_path),
        "unexpected outputS3Uri: {output_s3_uri}"
    );

    Ok(())
}

async fn wait_for_execution_success(execution_arn: &str) -> Result<(), Box<dyn Error>> {
    let sdk_config = aws_config::defaults(BehaviorVersion::latest())
        .profile_name("admin")
        .load()
        .await;
    let client = SfnClient::new(&sdk_config);

    for _ in 0..300 {
        let execution = client
            .describe_execution()
            .execution_arn(execution_arn)
            .send()
            .await?;
        match execution.status().as_str() {
            "SUCCEEDED" => return Ok(()),
            "FAILED" | "TIMED_OUT" | "ABORTED" => {
                return Err(format!(
                    "workflow {execution_arn} ended as {}: {}: {}",
                    execution.status().as_str(),
                    execution.error().unwrap_or("unknown error"),
                    execution.cause().unwrap_or("no cause"),
                )
                .into());
            }
            _ => tokio::time::sleep(Duration::from_secs(2)).await,
        }
    }

    Err(format!("workflow {execution_arn} did not complete within 10 minutes").into())
}
