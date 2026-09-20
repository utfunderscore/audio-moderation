mod common;

use database::{NewPipelineTask, NewReviewJob, PipelineTaskStore, ReviewJobStatus, ReviewJobStore};

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
async fn details_expose_only_the_same_tenant_linked_evaluation() {
    let database = TestDatabase::start().await;
    let reviews = ReviewJobStore::new(database.pool.clone());
    let pipelines = PipelineTaskStore::new(database.pool.clone());
    let review = reviews
        .create_or_get(NewReviewJob {
            tenant_id: "tenant-a",
            idempotency_key: Some("request-1"),
        })
        .await
        .unwrap();

    let before = reviews
        .get_details(review.job_id, "tenant-a")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.evaluation_id, None);

    let caller_reference = format!("review-job:{}", review.job_id);
    let pipeline = pipelines
        .create_or_get(NewPipelineTask {
            tenant_id: "tenant-a",
            idempotency_key: "review-upload:1",
            caller_reference: Some(&caller_reference),
            audio_s3_uris: &["s3://uploads/reviews/1/source".to_owned()],
        })
        .await
        .unwrap();
    let after = reviews
        .get_details(review.job_id, "tenant-a")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        after.evaluation_id.unwrap().to_string(),
        pipeline.evaluation_id
    );
    assert!(
        reviews
            .get_details(review.job_id, "tenant-b")
            .await
            .unwrap()
            .is_none()
    );
}
