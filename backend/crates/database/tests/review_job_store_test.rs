mod common;

use database::{
    NewPipelineTask, NewReviewJob, NewReviewPipelineTask, PipelineTaskError,
    PipelineTaskEventStore, PipelineTaskEventTicketStore, PipelineTaskStore, ReviewJobStatus,
    ReviewJobStore,
};

use common::TestDatabase;

const TOKEN_HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_TOKEN_HASH: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[tokio::test]
async fn creates_serial_job_and_replays_by_tenant_idempotency_key() {
    let database = TestDatabase::start().await;
    let store = ReviewJobStore::new(database.pool.clone());

    let created = store
        .create_or_get(NewReviewJob {
            tenant_id: "tenant-a",
            idempotency_key: Some("request-1"),
            access_token_hash: TOKEN_HASH,
        })
        .await
        .unwrap();
    let replayed = store
        .create_or_get(NewReviewJob {
            tenant_id: "tenant-a",
            idempotency_key: Some("request-1"),
            access_token_hash: TOKEN_HASH,
        })
        .await
        .unwrap();
    let other_tenant = store
        .create_or_get(NewReviewJob {
            tenant_id: "tenant-b",
            idempotency_key: Some("request-1"),
            access_token_hash: TOKEN_HASH,
        })
        .await
        .unwrap();

    assert!(created.created);
    assert_eq!(created.job_id, 1);
    assert_eq!(created.input_file_path, "reviews/1/source");
    assert_eq!(created.access_token_hash, TOKEN_HASH);
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
            access_token_hash: TOKEN_HASH,
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
    let review = tasks
        .create_or_get_review(NewReviewPipelineTask {
            tenant_id: "tenant-a",
            idempotency_key: "request-1",
            access_token_hash: TOKEN_HASH,
            uploads_bucket: "uploads",
        })
        .await
        .unwrap();

    let details = reviews
        .get_details(review.job.job_id, "tenant-a")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        details.evaluation_id.unwrap().to_string(),
        review.task.evaluation_id
    );
}

#[tokio::test]
async fn direct_tasks_cannot_attach_to_reviews() {
    let database = TestDatabase::start().await;
    let reviews = ReviewJobStore::new(database.pool.clone());
    let tasks = PipelineTaskStore::new(database.pool.clone());
    let review = reviews
        .create_or_get(NewReviewJob {
            tenant_id: "tenant-a",
            idempotency_key: Some("review-request-direct-task"),
            access_token_hash: TOKEN_HASH,
        })
        .await
        .unwrap();
    let reference = format!("review-job:{}", review.job_id);

    let direct = tasks
        .create_or_get(NewPipelineTask {
            tenant_id: "tenant-a",
            idempotency_key: "direct-request",
            caller_reference: Some(&reference),
            audio_s3_uris: &["s3://uploads/direct.wav".to_owned()],
        })
        .await
        .unwrap();

    assert!(direct.review_job_id.is_none());
    assert!(
        tasks
            .get_by_review_job(review.job_id, "tenant-a")
            .await
            .unwrap()
            .is_none()
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
            access_token_hash: TOKEN_HASH,
            uploads_bucket: "uploads",
        })
        .await
        .unwrap();
    let replayed = tasks
        .create_or_get_review(NewReviewPipelineTask {
            tenant_id: "tenant-a",
            idempotency_key: "review-request-1",
            access_token_hash: OTHER_TOKEN_HASH,
            uploads_bucket: "uploads",
        })
        .await
        .unwrap();

    assert!(created.job.created);
    assert!(created.task.created);
    assert_eq!(created.job.job_id, replayed.job.job_id);
    assert_eq!(created.job.access_token_hash, TOKEN_HASH);
    assert_eq!(replayed.job.access_token_hash, TOKEN_HASH);
    assert_eq!(created.task.task_id, replayed.task.task_id);
    assert_eq!(created.task.evaluation_id, replayed.task.evaluation_id);
    assert_eq!(
        ReviewJobStore::new(database.pool.clone())
            .get_by_evaluation_id(&created.task.evaluation_id.parse().unwrap(), "tenant-a",)
            .await
            .unwrap()
            .unwrap()
            .access_token_hash,
        TOKEN_HASH
    );
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

#[tokio::test]
async fn rejects_a_corrupt_existing_review_task_mapping_with_a_typed_error() {
    let database = TestDatabase::start().await;
    let reviews = ReviewJobStore::new(database.pool.clone());
    let tasks = PipelineTaskStore::new(database.pool.clone());
    let review = reviews
        .create_or_get(NewReviewJob {
            tenant_id: "tenant-a",
            idempotency_key: Some("request-corrupt"),
            access_token_hash: TOKEN_HASH,
        })
        .await
        .unwrap();

    // Simulate a pre-existing bad mapping from before review links were kept
    // behind create_or_get_review.
    sqlx::query(
        "INSERT INTO pipeline_tasks (tenant_id, review_job_id, idempotency_key, caller_reference) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind("tenant-a")
    .bind(review.job_id)
    .bind("legacy-review-task")
    .bind("legacy-reference")
    .execute(&database.pool)
    .await
    .unwrap();

    assert!(matches!(
        tasks
            .create_or_get_review(NewReviewPipelineTask {
                tenant_id: "tenant-a",
                idempotency_key: "request-corrupt",
                uploads_bucket: "uploads",
            })
            .await,
        Err(PipelineTaskError::ReviewTaskConflict { review_job_id }) if review_job_id == review.job_id
    ));
}
