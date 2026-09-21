use std::env;
use std::error::Error;
use std::io::{Error as IoError, ErrorKind};
use std::time::{Duration, Instant};

use aws_config::BehaviorVersion;
use chrono::Duration as ChronoDuration;
use database::{
    NewPipelineTask, NewPipelineTaskEvent, PipelineTaskEventStore, PipelineTaskEventTicketStore,
    PipelineTaskStore, PipelineTaskWebSocketConnectionStore,
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use task_event_emitter::TaskEventEmitter;
use uuid::Uuid;

#[path = "../../../tests/support/task_events_websocket.rs"]
mod task_events_websocket;

use task_events_websocket::TaskEventsWebSocket;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const EVENT_TIMEOUT: Duration = Duration::from_secs(20);
const SUBSCRIPTION_TIMEOUT: Duration = Duration::from_secs(20);
const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const PRE_EXISTING_EVENT: &str = "INTEGRATION_TEST_PRE_EXISTING";
const LIVE_EVENT: &str = "INTEGRATION_TEST_LIVE";

struct Environment {
    endpoint: String,
    tenant_id: String,
    pool: PgPool,
}

#[tokio::test]
#[ignore = "requires a deployed task-events WebSocket API and PostgreSQL"]
async fn replays_live_delivers_and_cleans_up_task_events()
-> Result<(), Box<dyn Error + Send + Sync>> {
    let environment = Environment::load().await?;
    let task = PipelineTaskStore::new(environment.pool.clone())
        .create_or_get(NewPipelineTask {
            tenant_id: &environment.tenant_id,
            review_job_id: None,
            idempotency_key: &format!("deployed-task-events-{}", Uuid::new_v4()),
            caller_reference: None,
            audio_s3_uris: &["s3://integration-test-inputs/task-events.wav".to_owned()],
        })
        .await?;
    let task_id = task.task_id;
    println!("task-events integration fixture pipelineTaskId: {task_id}");

    let result = async {
        let events = PipelineTaskEventStore::new(environment.pool.clone());
        events
            .create(NewPipelineTaskEvent {
                task_id,
                event_name: PRE_EXISTING_EVENT,
            })
            .await?;

        let tickets = PipelineTaskEventTicketStore::new(environment.pool.clone());
        let ticket = format!("deployed-ticket-{}", Uuid::new_v4());
        tickets
            .create(task_id, &ticket, ChronoDuration::minutes(1))
            .await?;

        let mut socket = TaskEventsWebSocket::connect_and_subscribe(
            &environment.endpoint,
            &ticket,
            CONNECT_TIMEOUT,
        )
        .await?;
        assert_event_sequence(&mut socket, &[PRE_EXISTING_EVENT]).await?;

        let connection_id = wait_for_subscription(&environment.pool, task_id).await?;
        let sdk_config = aws_config::defaults(BehaviorVersion::latest())
            .profile_name("admin")
            .load()
            .await;
        let emitter = TaskEventEmitter::new(
            events.clone(),
            PipelineTaskWebSocketConnectionStore::new(environment.pool.clone()),
            &sdk_config,
            management_endpoint(&environment.endpoint)?,
        );
        let emission = emitter
            .emit(task_id, LIVE_EVENT)
            .await
            .map_err(|error| IoError::other(error.to_string()))?;
        assert!(
            emission.delivered >= 1,
            "the subscribed socket must receive the live event: {emission:?}"
        );
        assert_event_sequence(&mut socket, &[LIVE_EVENT]).await?;

        // The standalone suite owns strict close/disconnect cleanup coverage.
        // Still wait for persisted cleanup if the close handshake reports an
        // error so its diagnostic is not hidden by an early return.
        let close_result = socket.close_cleanly().await;
        let disconnect_result = wait_for_disconnect(&environment.pool, &connection_id).await;
        close_result?;
        disconnect_result?;

        // There are no concurrent producers during this replay. The expected
        // ordered history must nevertheless tolerate at-least-once frames.
        let replay_ticket = format!("deployed-ticket-{}", Uuid::new_v4());
        tickets
            .create(task_id, &replay_ticket, ChronoDuration::minutes(1))
            .await?;
        let mut replay_socket = TaskEventsWebSocket::connect_and_subscribe(
            &environment.endpoint,
            &replay_ticket,
            CONNECT_TIMEOUT,
        )
        .await?;
        assert_event_sequence(&mut replay_socket, &[PRE_EXISTING_EVENT, LIVE_EVENT]).await?;
        let replay_connection_id = wait_for_subscription(&environment.pool, task_id).await?;
        let replay_close_result = replay_socket.close_cleanly().await;
        let replay_disconnect_result =
            wait_for_disconnect(&environment.pool, &replay_connection_id).await;
        replay_close_result?;
        replay_disconnect_result?;
        Ok::<_, Box<dyn Error + Send + Sync>>(())
    }
    .await;

    if result.is_ok() {
        sqlx::query("DELETE FROM pipeline_tasks WHERE task_id = $1")
            .bind(task_id)
            .execute(&environment.pool)
            .await?;
    } else {
        // Retain failed fixtures and their durable history for diagnosis.
        println!("retaining failed task-events fixture pipelineTaskId: {task_id}");
    }
    result
}

impl Environment {
    async fn load() -> Result<Self, Box<dyn Error + Send + Sync>> {
        let endpoint = required_env("AUDIO_MODERATION_TASK_EVENTS_ENDPOINT")?;
        if !endpoint.starts_with("wss://") {
            return Err(invalid_input(
                "AUDIO_MODERATION_TASK_EVENTS_ENDPOINT must be a wss:// URL",
            ));
        }
        let tenant_id = required_env("AUDIO_MODERATION_TENANT_ID")?;
        let database_url = required_env("DATABASE_URL")?;
        let pool = PgPoolOptions::new()
            .max_connections(3)
            .connect(&database_url)
            .await?;
        Ok(Self {
            endpoint,
            tenant_id,
            pool,
        })
    }
}

async fn assert_event_sequence(
    socket: &mut TaskEventsWebSocket,
    expected: &[&str],
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let deadline = Instant::now() + EVENT_TIMEOUT;
    let mut expected_index = 0;
    while expected_index < expected.len() {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| {
                IoError::new(ErrorKind::TimedOut, "timed out waiting for task events")
            })?;
        let received = socket.next_event(remaining).await?;
        match expected.iter().position(|event| *event == received) {
            Some(index) if index == expected_index => expected_index += 1,
            Some(index) if index < expected_index => {
                // Fanout is at least once. Duplicate durable events are valid,
                // including when a replay and live fanout meet. The first
                // occurrence of each name must still preserve history order.
            }
            _ => {
                return Err(invalid_input(&format!(
                    "unexpected task event {received:?}; expected ordered history {expected:?}"
                )));
            }
        }
    }
    Ok(())
}

async fn wait_for_subscription(
    pool: &PgPool,
    task_id: i32,
) -> Result<String, Box<dyn Error + Send + Sync>> {
    let deadline = Instant::now() + SUBSCRIPTION_TIMEOUT;
    loop {
        if let Some(connection_id) = sqlx::query_scalar(
            "SELECT connection_id FROM pipeline_task_websocket_connections WHERE task_id = $1",
        )
        .bind(task_id)
        .fetch_optional(pool)
        .await?
        {
            return Ok(connection_id);
        }
        if Instant::now() >= deadline {
            return Err(IoError::new(
                ErrorKind::TimedOut,
                "WebSocket subscription was not persisted before the timeout",
            )
            .into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn wait_for_disconnect(
    pool: &PgPool,
    connection_id: &str,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let deadline = Instant::now() + DISCONNECT_TIMEOUT;
    loop {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pipeline_task_websocket_connections WHERE connection_id = $1)",
        )
        .bind(connection_id)
        .fetch_one(pool)
        .await?;
        if !exists {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(IoError::new(
                ErrorKind::TimedOut,
                "WebSocket disconnect cleanup was not persisted before the timeout",
            )
            .into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn management_endpoint(websocket_endpoint: &str) -> Result<String, Box<dyn Error + Send + Sync>> {
    websocket_endpoint
        .strip_prefix("wss://")
        .map(|endpoint| format!("https://{endpoint}"))
        .ok_or_else(|| invalid_input("task-events endpoint must be a wss:// URL"))
}

fn required_env(name: &str) -> Result<String, Box<dyn Error + Send + Sync>> {
    let value = env::var(name)?.trim().to_owned();
    if value.is_empty() {
        return Err(invalid_input(&format!("{name} must not be blank")));
    }
    Ok(value)
}

fn invalid_input(message: &str) -> Box<dyn Error + Send + Sync> {
    IoError::new(ErrorKind::InvalidInput, message.to_owned()).into()
}
