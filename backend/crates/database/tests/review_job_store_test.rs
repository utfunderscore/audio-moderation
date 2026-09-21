mod common;

use database::{
    NewPipelineTask, NewReviewJob, NewReviewPipelineTask, PipelineTaskEventStore,
    PipelineTaskEventTicketStore, PipelineTaskStore, ReviewJobStatus, ReviewJobStore,
};

use common::TestDatabase;

#[tokio::test]
async fn creates_serial_job_and_replays_by_tenant_idempotency_key() {
    let database = TestDatabase::start().await;
    let store = ReviewJobStore::new(database.pool.clone());

    let created = store
        .create_or_get(NewReviewJob {
            tenant_id: "tenant-a",
            idempotency_key: Some("request-1"),
        })
        .await
        .unwrap();
    let replayed = store
        .create_or_get(NewReviewJob {
            tenant_id: "tenant-a",
            idempotency_key: Some("request-1"),
        })
        .await
        .unwrap();
    let other_tenant = store
        .create_or_get(NewReviewJob {
            tenant_id: "tenant-b",
            idempotency_key: Some("request-1"),
        })
        .await
        .unwrap();

    assert!(created.created);
    assert_eq!(created.job_id, 1);
    assert_eq!(created.input_file_path, "reviews/1/source");
    assert!(!replayed.created);
    assert_eq!(replayed.job_id, created.job_id);
    assert_ne!(other_tenant.job_id, created.job_id);
    assert!(
        store
            .get(created.job_id, "tenant-b")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn upload_completion_is_idempotent_and_does_not_move_status_backwards() {
    let database = TestDatabase::start().await;
    let store = ReviewJobStore::new(database.pool.clone());
    let job = store
        .create_or_get(NewReviewJob {
            tenant_id: "tenant-a",
            idempotency_key: Some("request-1"),
        })
        .await
        .unwrap();

    let uploaded = store
        .mark_upload_complete(&job.input_file_path, "tenant-a")
        .await
        .unwrap()
        .unwrap();
    let processing = store
        .update_status(job.job_id, "tenant-a", ReviewJobStatus::Processing)
        .await
        .unwrap()
        .unwrap();
    let repeated = store
        .mark_upload_complete(&job.input_file_path, "tenant-a")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(uploaded.status, ReviewJobStatus::PendingProcessing);
    assert_eq!(processing.status, ReviewJobStatus::Processing);
    assert_eq!(repeated.status, ReviewJobStatus::Processing);
    assert_eq!(repeated.updated_at, processing.updated_at);
    assert!(
        store
            .mark_upload_complete(&job.input_file_path, "tenant-b")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn retrieves_the_pipeline_evaluation_linked_by_review_job_foreign_key() {
    let database = TestDatabase::start().await;
    let reviews = ReviewJobStore::new(database.pool.clone());
    let tasks = PipelineTaskStore::new(database.pool.clone());
    let review = reviews
        .create_or_get(NewReviewJob {
            tenant_id: "tenant-a",
            idempotency_key: Some("request-1"),
        })
        .await
        .unwrap();
    let task = tasks
        .create_or_get(NewPipelineTask {
            tenant_id: "tenant-a",
            review_job_id: Some(review.job_id),
            idempotency_key: "review-upload:1",
            caller_reference: Some("review-job:1"),
            audio_s3_uris: &["s3://uploads/reviews/1/source".to_owned()],
        })
        .await
        .unwrap();

    let details = reviews
        .get_details(review.job_id, "tenant-a")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        details.evaluation_id.unwrap().to_string(),
        task.evaluation_id
    );
}

#[tokio::test]
async fn atomically_creates_and_replays_a_review_pipeline_with_initial_event() {
    let database = TestDatabase::start().await;
    let tasks = PipelineTaskStore::new(database.pool.clone());
    let events = PipelineTaskEventStore::new(database.pool.clone());
    let tickets = PipelineTaskEventTicketStore::new(database.pool.clone());

    let created = tasks
        .create_or_get_review(NewReviewPipelineTask {
            tenant_id: "tenant-a",
            idempotency_key: "review-request-1",
            uploads_bucket: "uploads",
        })
        .await
        .unwrap();
    let replayed = tasks
        .create_or_get_review(NewReviewPipelineTask {
            tenant_id: "tenant-a",
            idempotency_key: "review-request-1",
            uploads_bucket: "uploads",
        })
        .await
        .unwrap();

    assert!(created.job.created);
    assert!(created.task.created);
    assert_eq!(created.job.job_id, replayed.job.job_id);
    assert_eq!(created.task.task_id, replayed.task.task_id);
    assert_eq!(created.task.evaluation_id, replayed.task.evaluation_id);
    assert_eq!(
        created.task.audio_s3_uris,
        vec![format!("s3://uploads/{}", created.job.input_file_path)]
    );
    assert_eq!(
        events
            .list(created.task.task_id)
            .await
            .unwrap()
            .iter()
            .map(|event| event.event_name.as_str())
            .collect::<Vec<_>>(),
        ["EVALUATION_ACCEPTED"]
    );
    let issued = tickets
        .create(
            created.task.task_id,
            "ticket-before-upload",
            chrono::Duration::minutes(2),
        )
        .await
        .unwrap();
    assert!(issued.expires_at > chrono::Utc::now());
    assert_eq!(
        tickets
            .consume("ticket-before-upload")
            .await
            .unwrap()
            .task_id,
        created.task.task_id
    );
}
