mod common;

use database::{NewPipelineTask, NewPipelineTaskEvent, PipelineTaskEventStore, PipelineTaskStore};

use common::TestDatabase;

#[tokio::test]
async fn records_and_replays_events_in_creation_order() {
    let database = TestDatabase::start().await;
    let task_store = PipelineTaskStore::new(database.pool.clone());
    let event_store = PipelineTaskEventStore::new(database.pool.clone());
    let task = task_store
        .create_or_get(NewPipelineTask {
            tenant_id: "tenant-a",
            idempotency_key: "request-1",
            caller_reference: None,
            audio_s3_uris: &["s3://uploads/audio.wav".to_owned()],
        })
        .await
        .unwrap();

    let accepted = event_store
        .create(NewPipelineTaskEvent {
            task_id: task.task_id,
            event_name: "EVALUATION_ACCEPTED",
        })
        .await
        .unwrap();
    let started = event_store
        .create(NewPipelineTaskEvent {
            task_id: task.task_id,
            event_name: "AUDIO_PROCESSING_STARTED",
        })
        .await
        .unwrap();

    let events = event_store.list(task.task_id).await.unwrap();

    assert!(accepted.event_id < started.event_id);
    assert_eq!(
        events
            .iter()
            .map(|event| event.event_name.as_str())
            .collect::<Vec<_>>(),
        ["EVALUATION_ACCEPTED", "AUDIO_PROCESSING_STARTED"]
    );
}

#[tokio::test]
async fn lists_only_the_requested_tasks_events() {
    let database = TestDatabase::start().await;
    let task_store = PipelineTaskStore::new(database.pool.clone());
    let event_store = PipelineTaskEventStore::new(database.pool.clone());
    let first = task_store
        .create_or_get(NewPipelineTask {
            tenant_id: "tenant-a",
            idempotency_key: "request-1",
            caller_reference: None,
            audio_s3_uris: &["s3://uploads/first.wav".to_owned()],
        })
        .await
        .unwrap();
    let second = task_store
        .create_or_get(NewPipelineTask {
            tenant_id: "tenant-a",
            idempotency_key: "request-2",
            caller_reference: None,
            audio_s3_uris: &["s3://uploads/second.wav".to_owned()],
        })
        .await
        .unwrap();

    event_store
        .create(NewPipelineTaskEvent {
            task_id: first.task_id,
            event_name: "EVALUATION_ACCEPTED",
        })
        .await
        .unwrap();
    event_store
        .create(NewPipelineTaskEvent {
            task_id: second.task_id,
            event_name: "EVALUATION_ACCEPTED",
        })
        .await
        .unwrap();

    let events = event_store.list(first.task_id).await.unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].task_id, first.task_id);
    assert_eq!(events[0].event_name, "EVALUATION_ACCEPTED");
}
