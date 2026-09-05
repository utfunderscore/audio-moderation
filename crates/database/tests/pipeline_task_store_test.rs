mod common;

use chrono::{DateTime, Utc};
use database::{NewPipelineTask, PipelineTaskStore};

use common::TestDatabase;

#[tokio::test]
async fn creates_ordered_inputs_and_replays_by_tenant_idempotency_key() {
    let database = TestDatabase::start().await;
    let store = PipelineTaskStore::new(database.pool.clone());
    let inputs = vec![
        "s3://uploads/second.wav".to_owned(),
        "s3://uploads/first.wav".to_owned(),
    ];

    let created = store
        .create_or_get(NewPipelineTask {
            tenant_id: "tenant-a",
            idempotency_key: "request-1",
            caller_reference: Some("review-42"),
            audio_s3_uris: &inputs,
        })
        .await
        .unwrap();
    let replayed = store
        .create_or_get(NewPipelineTask {
            tenant_id: "tenant-a",
            idempotency_key: "request-1",
            caller_reference: Some("review-42"),
            audio_s3_uris: &["s3://uploads/different.wav".to_owned()],
        })
        .await
        .unwrap();

    assert!(created.created);
    assert_eq!(created.audio_s3_uris, inputs);
    assert_eq!(created.caller_reference.as_deref(), Some("review-42"));
    assert!(!replayed.created);
    assert_eq!(replayed.task_id, created.task_id);
    assert_eq!(replayed.audio_s3_uris, created.audio_s3_uris);
}

#[tokio::test]
async fn only_one_concurrent_dispatch_claim_succeeds() {
    let database = TestDatabase::start().await;
    let store = PipelineTaskStore::new(database.pool.clone());
    let task = store
        .create_or_get(NewPipelineTask {
            tenant_id: "tenant-a",
            idempotency_key: "request-1",
            caller_reference: None,
            audio_s3_uris: &["s3://uploads/audio.wav".to_owned()],
        })
        .await
        .unwrap();

    let (first, second) = tokio::join!(
        store.claim_dispatch(task.task_id),
        store.claim_dispatch(task.task_id)
    );
    let claims = [first.unwrap(), second.unwrap()];

    assert_eq!(claims.iter().filter(|claim| claim.is_some()).count(), 1);
    assert!(claims.contains(&Some(1)));
}

#[tokio::test]
async fn dispatch_failures_are_bounded_and_become_terminal() {
    let database = TestDatabase::start().await;
    let store = PipelineTaskStore::new(database.pool.clone());
    let task = store
        .create_or_get(NewPipelineTask {
            tenant_id: "tenant-a",
            idempotency_key: "request-1",
            caller_reference: None,
            audio_s3_uris: &["s3://uploads/audio.wav".to_owned()],
        })
        .await
        .unwrap();

    for attempt in 1..=3 {
        assert_eq!(
            store.claim_dispatch(task.task_id).await.unwrap(),
            Some(attempt)
        );
        store
            .record_dispatch_failure(task.task_id, "Step Functions unavailable")
            .await
            .unwrap();
    }

    assert_eq!(store.claim_dispatch(task.task_id).await.unwrap(), None);
    let row: (
        String,
        i32,
        Option<String>,
        Option<String>,
        Option<DateTime<Utc>>,
    ) = sqlx::query_as(
        r#"
            SELECT status::TEXT, attempt_count, error_code, error_message, completed_at
            FROM pipeline_tasks
            WHERE task_id = $1
        "#,
    )
    .bind(task.task_id)
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert_eq!(row.0, "FAILED");
    assert_eq!(row.1, 3);
    assert_eq!(row.2.as_deref(), Some("DISPATCH_FAILED"));
    assert_eq!(row.3.as_deref(), Some("Step Functions unavailable"));
    assert!(row.4.is_some());
}

#[tokio::test]
async fn recording_execution_releases_claim_and_clears_dispatch_error() {
    let database = TestDatabase::start().await;
    let store = PipelineTaskStore::new(database.pool.clone());
    let task = store
        .create_or_get(NewPipelineTask {
            tenant_id: "tenant-a",
            idempotency_key: "request-1",
            caller_reference: None,
            audio_s3_uris: &["s3://uploads/audio.wav".to_owned()],
        })
        .await
        .unwrap();
    store.claim_dispatch(task.task_id).await.unwrap().unwrap();
    store
        .record_dispatch_failure(task.task_id, "temporary failure")
        .await
        .unwrap();
    store.claim_dispatch(task.task_id).await.unwrap().unwrap();

    store
        .record_execution(task.task_id, "arn:aws:states:execution:42")
        .await
        .unwrap();

    assert_eq!(store.claim_dispatch(task.task_id).await.unwrap(), None);
    let row: (
        Option<String>,
        Option<DateTime<Utc>>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        r#"
            SELECT execution_arn, dispatch_started_at, error_code, error_message
            FROM pipeline_tasks
            WHERE task_id = $1
        "#,
    )
    .bind(task.task_id)
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert_eq!(row.0.as_deref(), Some("arn:aws:states:execution:42"));
    assert!(row.1.is_none());
    assert!(row.2.is_none());
    assert!(row.3.is_none());
}
