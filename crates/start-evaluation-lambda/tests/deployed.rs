use std::env;
use std::error::Error;
use std::io::{Error as IoError, ErrorKind};
use std::time::{Duration, Instant};

use aws_config::BehaviorVersion;
use aws_sdk_sfn::Client as SfnClient;
use database::{NewPipelineTask, PipelineTask, PipelineTaskStore};
use reqwest::StatusCode;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::sync::oneshot;
use uuid::Uuid;

#[path = "../../../tests/support/task_events_websocket.rs"]
mod task_events_websocket;

use task_events_websocket::TaskEventsWebSocket;

const START_EVALUATION_PATH: &str = "/audio.moderation.v1.AudioModerationService/StartEvaluation";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const WEBSOCKET_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const WEBSOCKET_EVENT_TIMEOUT: Duration = Duration::from_secs(20);
const SUCCESSFUL_LIFECYCLE_EVENTS: [&str; 8] = [
    "EVALUATION_ACCEPTED",
    "AUDIO_PROCESSING_STARTED",
    "AUDIO_PROCESSING_FINISHED",
    "ASR_STARTED",
    "ASR_FINISHED",
    "MODERATION_PROCESSING_STARTED",
    "MODERATION_PROCESSING_FINISHED",
    "SUCCEEDED",
];
const SYNTHETIC_AUDIO_S3_URIS: [&str; 2] = [
    "s3://integration-test-inputs/first.wav",
    "s3://integration-test-inputs/second.wav",
];

struct Environment {
    endpoint: String,
    task_events_endpoint: Option<String>,
    tenant_id: String,
    audio_s3_uris: Vec<String>,
    pool: PgPool,
}

struct StoredTask {
    task_id: i32,
    caller_reference: Option<String>,
    outcome: Option<String>,
    execution_arn: Option<String>,
    attempt_count: i32,
    active_lease: bool,
}

struct HttpResponse {
    status: StatusCode,
    body: String,
}

struct CollectedTaskEvents {
    observed: Vec<String>,
    delivery_error: Option<String>,
    close_error: Option<String>,
}

#[derive(sqlx::FromRow)]
struct ProcessingReportRow {
    outcome: Option<String>,
    audio_status: String,
    stitched_audio_s3_uri: Option<String>,
    audio_error_code: Option<String>,
    audio_error_message: Option<String>,
    transcription_status: String,
    transcription_task_id: Option<String>,
    transcription: Option<String>,
    transcription_error_code: Option<String>,
    transcription_error_message: Option<String>,
    moderation_status: String,
    moderation_task_id: Option<String>,
    sexual: Option<f64>,
    hate_or_discrimination: Option<f64>,
    harassment_or_abuse: Option<f64>,
    violence_or_threats: Option<f64>,
    asking_for_pii: Option<f64>,
    moderation_error_code: Option<String>,
    moderation_error_message: Option<String>,
}

#[tokio::test]
#[ignore = "requires a deployed start-evaluation Lambda and PostgreSQL"]
async fn evaluation_ingress_rejects_invalid_requests_without_writing_tasks()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let environment = Environment::load_with_synthetic_audio().await?;
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
-> Result<(), Box<dyn Error + Send + Sync>> {
    let environment = Environment::load_with_required_audio_and_task_events().await?;
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

    // The durable replay covers events emitted before this connection is
    // registered. Complete dispatch assertions first, then capture a setup
    // failure so workflow diagnostics still run.
    let socket_result = match environment.task_events_endpoint() {
        Ok(endpoint) => {
            TaskEventsWebSocket::connect_and_subscribe(
                endpoint,
                task.task_id,
                WEBSOCKET_CONNECT_TIMEOUT,
            )
            .await
        }
        Err(error) => Err(error),
    };
    let mut websocket_error = None;
    let mut collector = match socket_result {
        Ok(socket) => {
            let (stop_collector, collector_stop) = oneshot::channel();
            Some((
                stop_collector,
                tokio::spawn(collect_task_events(socket, collector_stop)),
            ))
        }
        Err(error) => {
            let message = format!("task-events setup failed: {error}");
            println!("{message}");
            websocket_error = Some(message);
            None
        }
    };
    let workflow_result =
        wait_for_execution_success(&environment.pool, task.task_id, execution_arn).await;
    // On success, let the collector drain through all expected names before
    // closing. This avoids racing a final already-delivered frame against the
    // cancellation signal. A bounded post-workflow wait still prevents a
    // missing WebSocket event from hanging the deployed test.
    let observed_events = if let Some((stop_collector, mut collector)) = collector.take() {
        let collector_result = if workflow_result.is_ok() {
            match tokio::time::timeout(WEBSOCKET_EVENT_TIMEOUT, &mut collector).await {
                Ok(result) => result,
                Err(_) => {
                    let _ = stop_collector.send(());
                    collector.await
                }
            }
        } else {
            let _ = stop_collector.send(());
            collector.await
        };
        match collector_result {
            Ok(collection) => {
                if let Some(error) = collection.delivery_error {
                    let message = format!("task-events collector failed: {error}");
                    println!("{message}");
                    websocket_error.get_or_insert(message);
                }
                if let Some(error) = collection.close_error {
                    // E2E validates delivery; the standalone suite owns strict
                    // close/disconnect cleanup coverage.
                    println!("task-events collector close problem (best effort): {error}");
                }
                collection.observed
            }
            Err(error) => {
                let message = format!("task-events collector panicked: {error}");
                println!("{message}");
                websocket_error.get_or_insert(message);
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    let (persisted_events, persisted_events_error) =
        match persisted_event_names(&environment.pool, task.task_id).await {
            Ok(events) => (events, None),
            Err(error) => {
                let message = format!("could not query durable task-event history: {error}");
                println!("{message}");
                (Vec::new(), Some(message))
            }
        };
    let report_error = print_task_events_report(
        task.task_id,
        &SUCCESSFUL_LIFECYCLE_EVENTS,
        &observed_events,
        &persisted_events,
    )
    .err()
    .map(|error| {
        let message = format!("could not serialize task-events diagnostic report: {error}");
        println!("{message}");
        message
    });
    // Do not replace workflow diagnostics with a WebSocket setup, delivery, or
    // diagnostic-report failure.
    workflow_result?;
    if let Some(error) = websocket_error {
        return Err(IoError::other(error).into());
    }
    let observed_missing = missing_event_names(&observed_events, &SUCCESSFUL_LIFECYCLE_EVENTS);
    if !observed_missing.is_empty() {
        return Err(IoError::other(format!(
            "live subscription missed lifecycle events {observed_missing:?}; expected {SUCCESSFUL_LIFECYCLE_EVENTS:?}; observed {observed_events:?}"
        ))
        .into());
    }
    if let Some(error) = persisted_events_error {
        return Err(IoError::other(error).into());
    }
    if let Some(error) = report_error {
        return Err(IoError::other(error).into());
    }
    assert_event_names_observed(
        &observed_events,
        &SUCCESSFUL_LIFECYCLE_EVENTS,
        "live subscription",
    );
    assert_event_names_observed(
        &persisted_events,
        &SUCCESSFUL_LIFECYCLE_EVENTS,
        "durable event history",
    );

    // A terminal workflow has no concurrent event producer. At-least-once
    // delivery can still duplicate frames, so require every durable name
    // rather than an exact frame count or a replay/live boundary ordering.
    let mut replay_socket = TaskEventsWebSocket::connect_and_subscribe(
        environment.task_events_endpoint()?,
        task.task_id,
        WEBSOCKET_CONNECT_TIMEOUT,
    )
    .await?;
    let replayed_events =
        collect_expected_events(&mut replay_socket, &SUCCESSFUL_LIFECYCLE_EVENTS).await?;
    if let Err(error) = replay_socket.close_cleanly().await {
        // The terminal replay was already fully observed. The standalone suite
        // asserts disconnect cleanup, so closing is best effort here.
        println!("task-events terminal replay close problem (best effort): {error}");
    }
    print_task_events_report(
        task.task_id,
        &SUCCESSFUL_LIFECYCLE_EVENTS,
        &replayed_events,
        &persisted_events,
    )?;
    assert_event_names_observed(
        &replayed_events,
        &SUCCESSFUL_LIFECYCLE_EVENTS,
        "terminal replay",
    );

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
async fn evaluation_dispatch_dispatches_a_seeded_undispatched_task()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let environment = Environment::load_with_required_audio().await?;
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
async fn evaluation_dispatch_retries_a_seeded_failed_dispatch()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let environment = Environment::load_with_required_audio().await?;
    let client = http_client()?;
    let key = unique_key("retry");
    let caller_reference = unique_key("caller");
    let store = PipelineTaskStore::new(environment.pool.clone());
    let seeded = seed_task(&store, &environment, &key, &caller_reference).await?;
    assert_eq!(store.claim_dispatch(seeded.task_id).await?, Some(1));
    store.record_dispatch_failure(seeded.task_id).await?;

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
#[ignore = "requires a deployed start-evaluation Lambda and PostgreSQL"]
async fn evaluation_ingress_skips_active_leases_and_terminal_failed_tasks()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let environment = Environment::load_with_synthetic_audio().await?;
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
        store.record_dispatch_failure(failed.task_id).await?;
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
    assert_eq!(failed_after.outcome.as_deref(), Some("FAILED"));
    assert_eq!(failed_after.attempt_count, 3);
    assert!(!failed_after.active_lease);
    assert!(failed_after.execution_arn.is_none());

    Ok(())
}

#[tokio::test]
#[ignore = "requires a deployed start-evaluation Lambda and PostgreSQL"]
async fn evaluation_ingress_rejects_conflicting_payloads_for_a_seeded_leased_task()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let environment = Environment::load_with_synthetic_audio().await?;
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
    async fn load_with_synthetic_audio() -> Result<Self, Box<dyn Error + Send + Sync>> {
        Self::load(false, false).await
    }

    async fn load_with_required_audio() -> Result<Self, Box<dyn Error + Send + Sync>> {
        Self::load(true, false).await
    }

    async fn load_with_required_audio_and_task_events() -> Result<Self, Box<dyn Error + Send + Sync>>
    {
        Self::load(true, true).await
    }

    async fn load(
        require_audio_fixture: bool,
        require_task_events: bool,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let endpoint = required_env("AUDIO_MODERATION_API_ENDPOINT")?;
        let task_events_endpoint = match env::var("AUDIO_MODERATION_TASK_EVENTS_ENDPOINT") {
            Ok(value) => {
                let value = value.trim().to_owned();
                if !value.starts_with("wss://") {
                    return Err(invalid_input(
                        "AUDIO_MODERATION_TASK_EVENTS_ENDPOINT must be a wss:// URL",
                    ));
                }
                Some(value)
            }
            Err(env::VarError::NotPresent) if require_task_events => {
                return Err(invalid_input(
                    "AUDIO_MODERATION_TASK_EVENTS_ENDPOINT is required for WebSocket assertions",
                ));
            }
            Err(env::VarError::NotPresent) => None,
            Err(error) => return Err(error.into()),
        };
        let tenant_id = required_env("AUDIO_MODERATION_TENANT_ID")?;
        let database_url = required_env("DATABASE_URL")?;
        let audio_s3_uris = match env::var("AUDIO_MODERATION_TEST_AUDIO_S3_URIS") {
            Ok(value) => {
                let audio_s3_uris: Vec<String> = serde_json::from_str(&value)?;
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
                audio_s3_uris
            }
            Err(env::VarError::NotPresent) if require_audio_fixture => {
                return Err(invalid_input(
                    "AUDIO_MODERATION_TEST_AUDIO_S3_URIS is required for tests that dispatch the deployed workflow",
                ));
            }
            Err(env::VarError::NotPresent) => SYNTHETIC_AUDIO_S3_URIS
                .iter()
                .map(|uri| (*uri).to_owned())
                .collect(),
            Err(error) => return Err(error.into()),
        };

        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(&database_url)
            .await?;
        Ok(Self {
            endpoint,
            task_events_endpoint,
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

    fn task_events_endpoint(&self) -> Result<&str, Box<dyn Error + Send + Sync>> {
        self.task_events_endpoint.as_deref().ok_or_else(|| {
            invalid_input(
                "AUDIO_MODERATION_TASK_EVENTS_ENDPOINT is required for WebSocket assertions",
            )
        })
    }
}

fn required_env(name: &str) -> Result<String, Box<dyn Error + Send + Sync>> {
    let value = env::var(name)?;
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err(invalid_input(&format!("{name} must not be blank")));
    }
    Ok(value)
}

fn invalid_input(message: &str) -> Box<dyn Error + Send + Sync> {
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

fn successful_json(response: HttpResponse) -> Result<Value, Box<dyn Error + Send + Sync>> {
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
) -> Result<PipelineTask, Box<dyn Error + Send + Sync>> {
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
    let row: (
        i32,
        Option<String>,
        Option<String>,
        Option<String>,
        i32,
        bool,
    ) = sqlx::query_as(
        r#"
            SELECT task_id, caller_reference, outcome::TEXT, execution_arn, attempt_count,
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
        outcome: row.2,
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
) -> Result<(), Box<dyn Error + Send + Sync>> {
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

async fn wait_for_execution_success(
    pool: &PgPool,
    task_id: i32,
    execution_arn: &str,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let sdk_config = aws_config::defaults(BehaviorVersion::latest())
        .profile_name("admin")
        .load()
        .await;
    let client = SfnClient::new(&sdk_config);
    let timeout = workflow_timeout();

    for _ in 0..(timeout.as_secs() / 2) {
        let execution = client
            .describe_execution()
            .execution_arn(execution_arn)
            .send()
            .await?;
        match execution.status().as_str() {
            "SUCCEEDED" => {
                let outcome = print_processing_report(
                    pool,
                    task_id,
                    execution_arn,
                    execution.status().as_str(),
                    execution.error(),
                    execution.cause(),
                    execution.output(),
                )
                .await?;
                if outcome.as_deref() == Some("SUCCEEDED") {
                    return Ok(());
                }
                return Err(format!(
                    "workflow {execution_arn} completed but pipeline task {task_id} ended as {}",
                    outcome.as_deref().unwrap_or("unknown")
                )
                .into());
            }
            "FAILED" | "TIMED_OUT" | "ABORTED" => {
                print_processing_report(
                    pool,
                    task_id,
                    execution_arn,
                    execution.status().as_str(),
                    execution.error(),
                    execution.cause(),
                    execution.output(),
                )
                .await?;
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

    print_processing_report(
        pool,
        task_id,
        execution_arn,
        "WAIT_TIMEOUT",
        Some("IntegrationTestTimeout"),
        Some("the workflow did not reach a terminal state before the test timeout"),
        None,
    )
    .await?;
    Err(format!(
        "workflow {execution_arn} did not complete within {} seconds",
        timeout.as_secs()
    )
    .into())
}

async fn collect_task_events(
    mut socket: TaskEventsWebSocket,
    mut stop: oneshot::Receiver<()>,
) -> CollectedTaskEvents {
    let mut observed = Vec::new();
    loop {
        tokio::select! {
            _ = &mut stop => {
                return CollectedTaskEvents {
                    observed,
                    delivery_error: None,
                    close_error: socket.close_cleanly().await.err().map(|error| error.to_string()),
                };
            }
            event = socket.next_event(WEBSOCKET_EVENT_TIMEOUT) => match event {
                Ok(event) => {
                    observed.push(event);
                    if contains_all_event_names(&observed, &SUCCESSFUL_LIFECYCLE_EVENTS) {
                        return CollectedTaskEvents {
                            observed,
                            delivery_error: None,
                            close_error: socket.close_cleanly().await.err().map(|error| error.to_string()),
                        };
                    }
                }
                // A quiet interval is normal while an external pipeline step is running.
                Err(error) if error.downcast_ref::<IoError>().is_some_and(|error| error.kind() == ErrorKind::TimedOut) => {},
                Err(error) => {
                    return CollectedTaskEvents {
                        observed,
                        delivery_error: Some(error.to_string()),
                        close_error: None,
                    };
                }
            },
        }
    }
}

async fn collect_expected_events(
    socket: &mut TaskEventsWebSocket,
    expected: &[&str],
) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
    let deadline = Instant::now() + WEBSOCKET_EVENT_TIMEOUT;
    let mut observed = Vec::new();
    while !expected
        .iter()
        .all(|expected_event| observed.iter().any(|event| event == expected_event))
    {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| {
                IoError::new(
                    ErrorKind::TimedOut,
                    "timed out waiting for replayed task events",
                )
            })?;
        observed.push(socket.next_event(remaining).await?);
    }
    Ok(observed)
}

async fn persisted_event_names(pool: &PgPool, task_id: i32) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT event_name FROM pipeline_task_events WHERE task_id = $1 ORDER BY event_id",
    )
    .bind(task_id)
    .fetch_all(pool)
    .await
}

fn assert_event_names_observed(observed: &[String], expected: &[&str], source: &str) {
    let missing = missing_event_names(observed, expected);
    assert!(
        missing.is_empty(),
        "{source} missed lifecycle events {missing:?}; expected {expected:?}; observed {observed:?}"
    );
}

fn missing_event_names<'a>(observed: &[String], expected: &[&'a str]) -> Vec<&'a str> {
    expected
        .iter()
        .copied()
        .filter(|expected_event| !observed.iter().any(|event| event == expected_event))
        .collect()
}

fn contains_all_event_names(observed: &[String], expected: &[&str]) -> bool {
    expected
        .iter()
        .all(|expected_event| observed.iter().any(|event| event == *expected_event))
}

fn print_task_events_report(
    task_id: i32,
    expected: &[&str],
    observed: &[String],
    persisted: &[String],
) -> Result<(), serde_json::Error> {
    println!(
        "Task-events result:\n{}",
        serde_json::to_string_pretty(&json!({
            "pipelineTaskId": task_id,
            "deliverySemantics": "at-least-once",
            "expected": expected,
            "observed": observed,
            "persisted": persisted,
        }))?
    );
    Ok(())
}

async fn print_processing_report(
    pool: &PgPool,
    task_id: i32,
    execution_arn: &str,
    execution_status: &str,
    execution_error: Option<&str>,
    execution_cause: Option<&str>,
    execution_output: Option<&str>,
) -> Result<Option<String>, Box<dyn Error + Send + Sync>> {
    let row = sqlx::query_as::<_, ProcessingReportRow>(
        r#"
            SELECT task.outcome::TEXT AS outcome,
                audio.status::TEXT AS audio_status,
                audio.stitched_audio_s3_uri,
                audio.error_code AS audio_error_code,
                audio.error_message AS audio_error_message,
                transcription.status::TEXT AS transcription_status,
                transcription.external_task_id AS transcription_task_id,
                transcription.transcript AS transcription,
                transcription.error_code AS transcription_error_code,
                transcription.error_message AS transcription_error_message,
                moderation.status::TEXT AS moderation_status,
                moderation.external_task_id AS moderation_task_id,
                moderation.sexual,
                moderation.hate_or_discrimination,
                moderation.harassment_or_abuse,
                moderation.violence_or_threats,
                moderation.asking_for_pii,
                moderation.error_code AS moderation_error_code,
                moderation.error_message AS moderation_error_message
            FROM pipeline_tasks task
            JOIN audio_processing_tasks audio USING (task_id)
            JOIN transcription_tasks transcription USING (task_id)
            JOIN moderation_tasks moderation USING (task_id)
            WHERE task.task_id = $1
        "#,
    )
    .bind(task_id)
    .fetch_one(pool)
    .await?;
    let outcome = row.outcome.clone();
    let input_files: Vec<String> = sqlx::query_scalar(
        "SELECT audio_s3_uri FROM pipeline_task_inputs WHERE task_id = $1 ORDER BY sequence",
    )
    .bind(task_id)
    .fetch_all(pool)
    .await?;
    let produced_files = row
        .stitched_audio_s3_uri
        .as_ref()
        .map(|uri| vec![json!({ "type": "stitchedAudio", "s3Uri": uri })])
        .unwrap_or_default();
    let moderation_scores = match (
        row.sexual,
        row.hate_or_discrimination,
        row.harassment_or_abuse,
        row.violence_or_threats,
        row.asking_for_pii,
    ) {
        (Some(sexual), Some(hate), Some(harassment), Some(violence), Some(pii)) => json!({
            "sexual": sexual,
            "hate_or_discrimination": hate,
            "harassment_or_abuse": harassment,
            "violence_or_threats": violence,
            "asking_for_pii": pii,
        }),
        _ => Value::Null,
    };
    let execution_output = execution_output
        .map(|output| serde_json::from_str(output).unwrap_or_else(|_| json!(output)))
        .unwrap_or(Value::Null);
    let report = json!({
        "pipelineTaskId": task_id,
        "pipelineOutcome": row.outcome,
        "execution": {
            "arn": execution_arn,
            "status": execution_status,
            "error": execution_error,
            "cause": execution_cause,
            "output": execution_output,
        },
        "inputFiles": input_files,
        "producedFiles": produced_files,
        "audioProcessing": {
            "status": row.audio_status,
            "stitchedAudioS3Uri": row.stitched_audio_s3_uri,
            "error": collected_error(row.audio_error_code, row.audio_error_message),
        },
        "transcription": {
            "status": row.transcription_status,
            "taskId": row.transcription_task_id,
            "result": row.transcription,
            "error": collected_error(
                row.transcription_error_code,
                row.transcription_error_message,
            ),
        },
        "moderation": {
            "status": row.moderation_status,
            "taskId": row.moderation_task_id,
            "scores": moderation_scores,
            "error": collected_error(row.moderation_error_code, row.moderation_error_message),
        },
    });

    println!(
        "Processing result:\n{}",
        serde_json::to_string_pretty(&report)?
    );
    Ok(outcome)
}

fn collected_error(code: Option<String>, message: Option<String>) -> Value {
    if code.is_none() && message.is_none() {
        Value::Null
    } else {
        json!({ "code": code, "message": message })
    }
}

fn workflow_timeout() -> Duration {
    env::var("AUDIO_MODERATION_WORKFLOW_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(600))
}

#[cfg(test)]
mod tests {
    use super::missing_event_names;

    #[test]
    fn missing_event_names_ignores_at_least_once_duplicates() {
        let observed = vec!["FIRST".to_owned(), "FIRST".to_owned(), "SECOND".to_owned()];

        assert_eq!(
            missing_event_names(&observed, &["FIRST", "SECOND"]),
            Vec::<&str>::new()
        );
        assert_eq!(
            missing_event_names(&observed, &["FIRST", "THIRD"]),
            ["THIRD"]
        );
    }
}
