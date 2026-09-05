mod common;

use database::{NewReviewJob, ReviewJobStatus, ReviewJobStore};

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
