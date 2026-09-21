mod common;

use chrono::Duration;
use database::{
    NewPipelineTask, PipelineTaskEventTicketError, PipelineTaskEventTicketStore, PipelineTaskStore,
};

use common::TestDatabase;

#[tokio::test]
async fn consumes_a_valid_ticket_only_once() {
    let database = TestDatabase::start().await;
    let task = PipelineTaskStore::new(database.pool.clone())
        .create_or_get(NewPipelineTask {
            tenant_id: "tenant-a",
            idempotency_key: "request-1",
            caller_reference: None,
            audio_s3_uris: &["s3://uploads/audio.wav".to_owned()],
        })
        .await
        .unwrap();
    let tickets = PipelineTaskEventTicketStore::new(database.pool);

    let created = tickets
        .create(task.task_id, "ticket-secret", Duration::minutes(1))
        .await
        .unwrap();
    assert_eq!(created.task_id, task.task_id);
    assert_eq!(
        tickets.consume("ticket-secret").await.unwrap().task_id,
        task.task_id
    );
    assert!(matches!(
        tickets.consume("ticket-secret").await,
        Err(PipelineTaskEventTicketError::Invalid)
    ));
}

#[tokio::test]
async fn rejects_expired_and_unknown_tickets() {
    let database = TestDatabase::start().await;
    let task = PipelineTaskStore::new(database.pool.clone())
        .create_or_get(NewPipelineTask {
            tenant_id: "tenant-a",
            idempotency_key: "request-1",
            caller_reference: None,
            audio_s3_uris: &["s3://uploads/audio.wav".to_owned()],
        })
        .await
        .unwrap();
    let pool = database.pool.clone();
    let tickets = PipelineTaskEventTicketStore::new(database.pool);

    tickets
        .create(task.task_id, "expired-ticket", Duration::minutes(1))
        .await
        .unwrap();
    sqlx::query(
        "UPDATE pipeline_task_event_tickets SET created_at = NOW() - INTERVAL '2 seconds', expires_at = NOW() - INTERVAL '1 second' WHERE task_id = $1",
    )
    .bind(task.task_id)
    .execute(&pool)
    .await
    .unwrap();

    assert!(matches!(
        tickets.consume("expired-ticket").await,
        Err(PipelineTaskEventTicketError::Invalid)
    ));
    assert!(matches!(
        tickets.consume("unknown-ticket").await,
        Err(PipelineTaskEventTicketError::Invalid)
    ));
}
