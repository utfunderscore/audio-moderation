mod common;

use database::{
    NewPipelineTask, NewPipelineTaskWebSocketConnection, PipelineTaskStore,
    PipelineTaskWebSocketConnectionStore,
};

use common::TestDatabase;

#[tokio::test]
async fn creates_subscriptions_idempotently_and_lists_a_tasks_connections() {
    let database = TestDatabase::start().await;
    let task_store = PipelineTaskStore::new(database.pool.clone());
    let connection_store = PipelineTaskWebSocketConnectionStore::new(database.pool.clone());
    let task = create_task(&task_store, "request-1").await;

    let created = connection_store
        .create_or_get(NewPipelineTaskWebSocketConnection {
            task_id: task.task_id,
            connection_id: "connection-b",
        })
        .await
        .unwrap();
    let replayed = connection_store
        .create_or_get(NewPipelineTaskWebSocketConnection {
            task_id: task.task_id,
            connection_id: "connection-b",
        })
        .await
        .unwrap();
    connection_store
        .create_or_get(NewPipelineTaskWebSocketConnection {
            task_id: task.task_id,
            connection_id: "connection-a",
        })
        .await
        .unwrap();

    let connections = connection_store.list(task.task_id).await.unwrap();

    assert!(created.created);
    assert!(!replayed.created);
    assert_eq!(replayed.created_at, created.created_at);
    assert_eq!(
        connections
            .iter()
            .map(|connection| connection.connection_id.as_str())
            .collect::<Vec<_>>(),
        ["connection-a", "connection-b"]
    );
}

#[tokio::test]
async fn rejects_a_second_task_stream_for_one_connection() {
    let database = TestDatabase::start().await;
    let task_store = PipelineTaskStore::new(database.pool.clone());
    let connection_store = PipelineTaskWebSocketConnectionStore::new(database.pool.clone());
    let first = create_task(&task_store, "request-1").await;
    let second = create_task(&task_store, "request-2").await;

    connection_store
        .create_or_get(NewPipelineTaskWebSocketConnection {
            task_id: first.task_id,
            connection_id: "connection-123",
        })
        .await
        .unwrap();
    let error = connection_store
        .create_or_get(NewPipelineTaskWebSocketConnection {
            task_id: second.task_id,
            connection_id: "connection-123",
        })
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        database::PipelineTaskWebSocketConnectionError::AlreadySubscribed { task_id }
            if task_id == first.task_id
    ));
    assert_eq!(connection_store.remove("connection-123").await.unwrap(), 1);
    assert!(
        connection_store
            .list(first.task_id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(connection_store.remove("connection-123").await.unwrap(), 0);
}

async fn create_task(store: &PipelineTaskStore, idempotency_key: &str) -> database::PipelineTask {
    store
        .create_or_get(NewPipelineTask {
            tenant_id: "tenant-a",
            idempotency_key,
            caller_reference: None,
            audio_s3_uris: &[format!("s3://uploads/{idempotency_key}.wav")],
        })
        .await
        .unwrap()
}
