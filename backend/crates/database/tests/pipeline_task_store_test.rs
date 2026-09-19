mod common;

use chrono::{DateTime, Utc};
use database::{
    DEFAULT_DISPATCH_LEASE, ModerationResult, NewPipelineTask, PipelineTaskError,
    PipelineTaskOutcome, PipelineTaskStatus, PipelineTaskStore, RecordExecutionResult,
};
use uuid::Uuid;

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
    assert_eq!(created.status, PipelineTaskStatus::Pending);
    assert_eq!(created.audio_s3_uris, inputs);
    assert_eq!(created.caller_reference.as_deref(), Some("review-42"));
    assert!(created.asr_task_id.is_none());
    assert!(!replayed.created);
    assert_eq!(replayed.task_id, created.task_id);
    assert_eq!(replayed.evaluation_id, created.evaluation_id);
    assert_eq!(created.evaluation_id.len(), 36);
    assert_eq!(replayed.audio_s3_uris, created.audio_s3_uris);

    let details = store
        .get_details(Uuid::parse_str(&created.evaluation_id).unwrap(), "tenant-a")
        .await
        .unwrap();
    assert_eq!(details.task_id, created.task_id);
    assert_eq!(details.audio_s3_uris, created.audio_s3_uris);
    assert!(matches!(
        store
            .get_details(Uuid::parse_str(&created.evaluation_id).unwrap(), "tenant-b")
            .await,
        Err(PipelineTaskError::NotFound)
    ));

    let step_statuses: (String, String, String) = sqlx::query_as(
        r#"
            SELECT audio.status::TEXT, transcription.status::TEXT, moderation.status::TEXT
            FROM audio_processing_tasks audio
            JOIN transcription_tasks transcription USING (task_id)
            JOIN moderation_tasks moderation USING (task_id)
            WHERE audio.task_id = $1
        "#,
    )
    .bind(created.task_id)
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert_eq!(
        step_statuses,
        ("PENDING".into(), "PENDING".into(), "PENDING".into())
    );
}

#[tokio::test]
async fn records_an_asr_task_id_once_and_allows_identical_retries() {
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

    store.record_asr_task(task.task_id, "fc-123").await.unwrap();
    store.record_asr_task(task.task_id, "fc-123").await.unwrap();

    let asr_task_id = sqlx::query_scalar::<_, Option<String>>(
        "SELECT external_task_id FROM transcription_tasks WHERE task_id = $1",
    )
    .bind(task.task_id)
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert_eq!(asr_task_id.as_deref(), Some("fc-123"));

    let replayed = store
        .create_or_get(NewPipelineTask {
            tenant_id: "tenant-a",
            idempotency_key: "request-1",
            caller_reference: None,
            audio_s3_uris: &["s3://uploads/audio.wav".to_owned()],
        })
        .await
        .unwrap();
    assert_eq!(replayed.asr_task_id.as_deref(), Some("fc-123"));
}

#[tokio::test]
async fn rejects_replacing_an_asr_task_id() {
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

    store.record_asr_task(task.task_id, "fc-123").await.unwrap();

    assert!(matches!(
        store.record_asr_task(task.task_id, "fc-456").await,
        Err(database::PipelineTaskError::AsrTaskConflict)
    ));
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
        store.record_dispatch_failure(task.task_id).await.unwrap();
    }

    assert_eq!(store.claim_dispatch(task.task_id).await.unwrap(), None);
    let row: (Option<String>, i32, Option<DateTime<Utc>>) = sqlx::query_as(
        r#"
            SELECT outcome::TEXT, attempt_count, completed_at
            FROM pipeline_tasks
            WHERE task_id = $1
        "#,
    )
    .bind(task.task_id)
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert_eq!(row.0.as_deref(), Some("FAILED"));
    assert_eq!(row.1, 3);
    assert!(row.2.is_some());
}

#[tokio::test]
async fn recording_execution_releases_claim() {
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
    store.record_dispatch_failure(task.task_id).await.unwrap();
    store.claim_dispatch(task.task_id).await.unwrap().unwrap();

    assert_eq!(
        store
            .record_execution(task.task_id, "arn:aws:states:execution:42")
            .await
            .unwrap(),
        RecordExecutionResult::Recorded
    );

    assert_eq!(store.claim_dispatch(task.task_id).await.unwrap(), None);
    let row: (Option<String>, Option<DateTime<Utc>>) = sqlx::query_as(
        r#"
            SELECT execution_arn, dispatch_started_at
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
}

#[tokio::test]
async fn records_the_same_execution_idempotently_and_rejects_conflicts() {
    let database = TestDatabase::start().await;
    let store = PipelineTaskStore::new(database.pool.clone());
    let task = new_task(&store).await;

    assert_eq!(
        store
            .record_execution(task.task_id, "arn:aws:states:execution:42")
            .await
            .unwrap(),
        RecordExecutionResult::Recorded
    );
    assert_eq!(
        store
            .record_execution(task.task_id, "arn:aws:states:execution:42")
            .await
            .unwrap(),
        RecordExecutionResult::AlreadyRecorded
    );
    assert!(matches!(
        store
            .record_execution(task.task_id, "arn:aws:states:execution:other")
            .await,
        Err(PipelineTaskError::ExecutionConflict { .. })
    ));
    assert!(matches!(
        store
            .record_execution(task.task_id + 1, "arn:aws:states:execution:42")
            .await,
        Err(PipelineTaskError::TaskNotFound(_))
    ));

    let terminal = store
        .create_or_get(NewPipelineTask {
            tenant_id: "tenant-lifecycle",
            idempotency_key: "request-terminal-execution",
            caller_reference: None,
            audio_s3_uris: &["s3://uploads/terminal.wav".to_owned()],
        })
        .await
        .unwrap();
    sqlx::query(
        "UPDATE pipeline_tasks SET outcome = 'FAILED', completed_at = NOW() WHERE task_id = $1",
    )
    .bind(terminal.task_id)
    .execute(&database.pool)
    .await
    .unwrap();
    assert!(matches!(
        store
            .record_execution(terminal.task_id, "arn:aws:states:execution:terminal")
            .await,
        Err(PipelineTaskError::ExecutionNotRecordable(_))
    ));
}

#[tokio::test]
async fn reclaims_an_expired_dispatch_lease() {
    let database = TestDatabase::start().await;
    let store = PipelineTaskStore::new(database.pool.clone());
    let task = new_task(&store).await;

    assert_eq!(store.claim_dispatch(task.task_id).await.unwrap(), Some(1));
    let expired_by_seconds = i64::try_from(DEFAULT_DISPATCH_LEASE.as_secs()).unwrap() + 1;
    sqlx::query(
        "UPDATE pipeline_tasks SET dispatch_started_at = NOW() - ($2 * INTERVAL '1 second') \
         WHERE task_id = $1",
    )
    .bind(task.task_id)
    .bind(expired_by_seconds)
    .execute(&database.pool)
    .await
    .unwrap();

    assert_eq!(store.claim_dispatch(task.task_id).await.unwrap(), Some(2));
}

#[tokio::test]
async fn persists_audio_and_asr_lifecycle_with_idempotent_retries() {
    let database = TestDatabase::start().await;
    let store = PipelineTaskStore::new(database.pool.clone());
    let task = new_task(&store).await;

    store.start_audio_processing(task.task_id).await.unwrap();
    store.start_audio_processing(task.task_id).await.unwrap();
    store
        .complete_audio_processing(task.task_id, "s3://artifacts/evaluations/1.wav")
        .await
        .unwrap();
    store
        .complete_audio_processing(task.task_id, "s3://artifacts/evaluations/1.wav")
        .await
        .unwrap();
    assert!(matches!(
        store
            .complete_audio_processing(task.task_id, "s3://artifacts/evaluations/other.wav")
            .await,
        Err(PipelineTaskError::AudioOutputConflict)
    ));

    store.start_asr(task.task_id, "token-1").await.unwrap();
    store.start_asr(task.task_id, "token-1").await.unwrap();
    store.record_asr_task(task.task_id, "fc-123").await.unwrap();
    store
        .complete_asr(task.task_id, "token-1", "fc-123", "hello world")
        .await
        .unwrap();
    store
        .complete_asr(task.task_id, "token-1", "fc-123", "hello world")
        .await
        .unwrap();
    assert!(matches!(
        store
            .complete_asr(task.task_id, "token-1", "fc-456", "hello world")
            .await,
        Err(PipelineTaskError::AsrTaskConflict)
    ));
    assert!(matches!(
        store
            .complete_asr(task.task_id, "token-1", "fc-123", "different")
            .await,
        Err(PipelineTaskError::TranscriptionConflict)
    ));

    let row: (String, String, String, String) = sqlx::query_as(
        r#"
            SELECT audio.status::TEXT, audio.stitched_audio_s3_uri,
                transcription.status::TEXT, transcription.transcript
            FROM pipeline_tasks task
            JOIN audio_processing_tasks audio USING (task_id)
            JOIN transcription_tasks transcription USING (task_id)
            WHERE task.task_id = $1
        "#,
    )
    .bind(task.task_id)
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert_eq!(row.0, "COMPLETED");
    assert_eq!(row.1, "s3://artifacts/evaluations/1.wav");
    assert_eq!(row.2, "COMPLETED");
    assert_eq!(row.3, "hello world");

    let replayed = new_task(&store).await;
    assert_eq!(replayed.status, PipelineTaskStatus::AsrFinished);
}

#[tokio::test]
async fn rejects_invalid_order_and_leaves_steps_pending() {
    let database = TestDatabase::start().await;
    let store = PipelineTaskStore::new(database.pool.clone());
    let task = new_task(&store).await;

    assert!(matches!(
        store.start_asr(task.task_id, "token").await,
        Err(PipelineTaskError::TransitionConflict { .. })
    ));
    assert!(matches!(
        store
            .complete_audio_processing(task.task_id, "s3://artifacts/1.wav")
            .await,
        Err(PipelineTaskError::TransitionConflict { .. })
    ));
    assert!(matches!(
        store
            .complete_asr(task.task_id, "token", "fc-123", "hello")
            .await,
        Err(PipelineTaskError::CallbackTokenNotFound)
    ));

    let steps: (String, Option<String>, String, Option<String>) = sqlx::query_as(
        "SELECT audio.status::TEXT, audio.stitched_audio_s3_uri, \
         transcription.status::TEXT, transcription.transcript \
         FROM audio_processing_tasks audio \
         JOIN transcription_tasks transcription USING (task_id) \
         WHERE audio.task_id = $1",
    )
    .bind(task.task_id)
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert_eq!(steps, ("PENDING".into(), None, "PENDING".into(), None));
}

#[tokio::test]
async fn terminal_tasks_do_not_regress() {
    let database = TestDatabase::start().await;
    let store = PipelineTaskStore::new(database.pool.clone());
    let task = new_task(&store).await;
    sqlx::query(
        "UPDATE pipeline_tasks SET outcome = 'FAILED', completed_at = NOW() WHERE task_id = $1",
    )
    .bind(task.task_id)
    .execute(&database.pool)
    .await
    .unwrap();

    for result in [
        store.start_audio_processing(task.task_id).await,
        store.start_asr(task.task_id, "token").await,
    ] {
        assert!(matches!(
            result,
            Err(PipelineTaskError::TransitionConflict { .. })
        ));
    }
    let outcome: String =
        sqlx::query_scalar("SELECT outcome::TEXT FROM pipeline_tasks WHERE task_id = $1")
            .bind(task.task_id)
            .fetch_one(&database.pool)
            .await
            .unwrap();
    assert_eq!(outcome, "FAILED");
}

#[tokio::test]
async fn callback_tokens_are_bound_to_their_task_and_allow_fast_identical_retries() {
    let database = TestDatabase::start().await;
    let store = PipelineTaskStore::new(database.pool.clone());
    let first = new_task(&store).await;
    let second = store
        .create_or_get(NewPipelineTask {
            tenant_id: "tenant-lifecycle",
            idempotency_key: "request-lifecycle-second",
            caller_reference: None,
            audio_s3_uris: &["s3://uploads/second.wav".to_owned()],
        })
        .await
        .unwrap();
    for task in [&first, &second] {
        store.start_audio_processing(task.task_id).await.unwrap();
        store
            .complete_audio_processing(task.task_id, "s3://artifacts/evaluations/audio.wav")
            .await
            .unwrap();
    }
    store.start_asr(first.task_id, "first-token").await.unwrap();
    store
        .start_asr(second.task_id, "second-token")
        .await
        .unwrap();

    assert!(matches!(
        store
            .complete_asr(first.task_id, "second-token", "fc-1", "hello")
            .await,
        Err(PipelineTaskError::CallbackTaskConflict)
    ));
    assert!(matches!(
        store
            .complete_asr(first.task_id, "missing-token", "fc-1", "hello")
            .await,
        Err(PipelineTaskError::CallbackTokenNotFound)
    ));

    // The callback may arrive before the external request response containing
    // its task ID. Persisting it first remains safe and record_asr_task can
    // subsequently fill in the same ID.
    store
        .complete_asr(first.task_id, "first-token", "fc-1", "hello")
        .await
        .unwrap();
    store.record_asr_task(first.task_id, "fc-1").await.unwrap();
    store
        .complete_asr(first.task_id, "first-token", "fc-1", "hello")
        .await
        .unwrap();
}

#[tokio::test]
async fn persists_moderation_with_step_bound_tokens_and_idempotent_retries() {
    let database = TestDatabase::start().await;
    let store = PipelineTaskStore::new(database.pool.clone());
    let task = new_task(&store).await;
    complete_asr(&store, task.task_id, "asr-token").await;

    store
        .start_moderation(task.task_id, "moderation-token")
        .await
        .unwrap();
    let callback_attempts: Vec<(String, String)> = sqlx::query_as(
        "SELECT step::TEXT, task_token_hash FROM pipeline_callback_attempts WHERE task_id = $1 ORDER BY step::TEXT",
    )
    .bind(task.task_id)
    .fetch_all(&database.pool)
    .await
    .unwrap();
    assert_eq!(callback_attempts.len(), 2);
    assert!(
        callback_attempts
            .iter()
            .all(|(_, digest)| digest.len() == 64
                && digest != "asr-token"
                && digest != "moderation-token")
    );
    assert!(matches!(
        store
            .finish_workflow(
                task.task_id,
                PipelineTaskOutcome::Succeeded,
                None,
                None,
                None
            )
            .await,
        Err(PipelineTaskError::TransitionConflict { .. })
    ));
    store
        .start_moderation(task.task_id, "moderation-token")
        .await
        .unwrap();
    let result = moderation_result();
    store
        .complete_moderation_for_callback(
            "moderation-token",
            task.task_id,
            "moderation-123",
            result,
        )
        .await
        .unwrap();
    store
        .complete_moderation_for_callback(
            "moderation-token",
            task.task_id,
            "moderation-123",
            result,
        )
        .await
        .unwrap();
    assert!(matches!(
        store
            .complete_moderation_for_callback(
                "moderation-token",
                task.task_id,
                "moderation-123",
                ModerationResult {
                    sexual: 0.03,
                    ..result
                },
            )
            .await,
        Err(PipelineTaskError::ModerationConflict)
    ));
    store
        .complete_moderation_for_callback(
            "moderation-token",
            task.task_id,
            "moderation-123",
            result,
        )
        .await
        .unwrap();
    assert!(matches!(
        store
            .complete_moderation_for_callback("asr-token", task.task_id, "moderation-123", result)
            .await,
        Err(PipelineTaskError::CallbackStepConflict)
    ));
    assert!(matches!(
        store
            .complete_asr(task.task_id, "moderation-token", "fc-123", "hello world")
            .await,
        Err(PipelineTaskError::CallbackStepConflict)
    ));

    assert!(matches!(
        store
            .complete_moderation_for_callback(
                "moderation-token",
                task.task_id,
                "moderation-456",
                result,
            )
            .await,
        Err(PipelineTaskError::ModerationConflict)
    ));
    assert!(matches!(
        store
            .complete_moderation_for_callback(
                "moderation-token",
                task.task_id + 1,
                "moderation-123",
                result,
            )
            .await,
        Err(PipelineTaskError::ModerationJobConflict)
    ));

    let persisted: (String, Option<String>, f64, f64, f64, f64, f64) = sqlx::query_as(
        "SELECT status::TEXT, external_task_id, sexual, hate_or_discrimination, harassment_or_abuse, \
          violence_or_threats, asking_for_pii FROM moderation_tasks WHERE task_id = $1",
    )
    .bind(task.task_id)
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert_eq!(
        persisted,
        (
            "COMPLETED".into(),
            Some("moderation-123".into()),
            0.02,
            0.15,
            0.08,
            0.01,
            0.42
        )
    );

    store
        .finish_workflow(
            task.task_id,
            PipelineTaskOutcome::Succeeded,
            None,
            None,
            None,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn records_moderation_task_ids_idempotently_and_rejects_conflicts() {
    let database = TestDatabase::start().await;
    let store = PipelineTaskStore::new(database.pool.clone());
    let task = new_task(&store).await;

    store
        .record_moderation_task(task.task_id, "moderation-123")
        .await
        .unwrap();
    store
        .record_moderation_task(task.task_id, "moderation-123")
        .await
        .unwrap();
    assert!(matches!(
        store
            .record_moderation_task(task.task_id, "moderation-456")
            .await,
        Err(PipelineTaskError::ModerationTaskConflict)
    ));
}

#[tokio::test]
async fn moderation_request_failure_is_terminal_and_idempotent() {
    let database = TestDatabase::start().await;
    let store = PipelineTaskStore::new(database.pool.clone());
    let task = new_task(&store).await;
    complete_asr(&store, task.task_id, "asr-token").await;
    store
        .start_moderation(task.task_id, "moderation-token")
        .await
        .unwrap();

    store
        .fail_moderation_request(task.task_id, "MODERATION_REQUEST_FAILED", "unavailable")
        .await
        .unwrap();
    store
        .fail_moderation_request(task.task_id, "MODERATION_REQUEST_FAILED", "unavailable")
        .await
        .unwrap();

    let persisted: (String, Option<String>, Option<String>, String) = sqlx::query_as(
        "SELECT moderation.status::TEXT, moderation.error_code, moderation.error_message, \
         task.outcome::TEXT FROM moderation_tasks moderation \
         JOIN pipeline_tasks task USING (task_id) WHERE task_id = $1",
    )
    .bind(task.task_id)
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert_eq!(
        persisted,
        (
            "FAILED".into(),
            Some("MODERATION_REQUEST_FAILED".into()),
            Some("unavailable".into()),
            "FAILED".into(),
        )
    );
}

#[tokio::test]
async fn moderation_failure_resolves_its_step_from_the_token_and_finalizes_afterwards() {
    let database = TestDatabase::start().await;
    let store = PipelineTaskStore::new(database.pool.clone());
    let task = new_task(&store).await;
    complete_asr(&store, task.task_id, "asr-token").await;
    store
        .start_moderation(task.task_id, "moderation-token")
        .await
        .unwrap();

    let attempt = store
        .record_callback_failure(
            "moderation-token",
            Some("MODERATION_FAILED"),
            Some("details"),
        )
        .await
        .unwrap();
    assert_eq!(attempt.task_id, task.task_id);
    store
        .record_callback_failure(
            "moderation-token",
            Some("MODERATION_FAILED"),
            Some("details"),
        )
        .await
        .unwrap();
    assert!(matches!(
        store
            .record_callback_failure("moderation-token", Some("MODERATION_FAILED"), Some("other"))
            .await,
        Err(PipelineTaskError::ModerationConflict)
    ));
    store
        .finalize_callback_failure("moderation-token")
        .await
        .unwrap();
    let outcome: String =
        sqlx::query_scalar("SELECT outcome::TEXT FROM pipeline_tasks WHERE task_id = $1")
            .bind(task.task_id)
            .fetch_one(&database.pool)
            .await
            .unwrap();
    assert_eq!(outcome, "FAILED");
}

#[tokio::test]
async fn asr_failure_rejects_a_moderation_token_without_mutating_moderation() {
    let database = TestDatabase::start().await;
    let store = PipelineTaskStore::new(database.pool.clone());
    let task = new_task(&store).await;
    complete_asr(&store, task.task_id, "asr-token").await;
    store
        .start_moderation(task.task_id, "moderation-token")
        .await
        .unwrap();

    assert!(matches!(
        store
            .record_asr_failure("moderation-token", Some("ASR_FAILED"), Some("details"))
            .await,
        Err(PipelineTaskError::CallbackStepConflict)
    ));
    let moderation: (String, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT status::TEXT, error_code, error_message FROM moderation_tasks WHERE task_id = $1",
    )
    .bind(task.task_id)
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert_eq!(moderation, ("PROCESSING".into(), None, None));
}

#[tokio::test]
async fn moderation_schema_rejects_out_of_range_and_non_finite_scores() {
    let database = TestDatabase::start().await;
    let store = PipelineTaskStore::new(database.pool.clone());
    let task = new_task(&store).await;

    assert!(
        sqlx::query("UPDATE moderation_tasks SET sexual = 1.01 WHERE task_id = $1")
            .bind(task.task_id)
            .execute(&database.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query(
            "UPDATE moderation_tasks SET sexual = 'NaN'::DOUBLE PRECISION WHERE task_id = $1",
        )
        .bind(task.task_id)
        .execute(&database.pool)
        .await
        .is_err()
    );
}

#[tokio::test]
async fn terminal_outcomes_are_idempotent_and_preserve_timeout_and_failure_diagnostics() {
    let database = TestDatabase::start().await;
    let store = PipelineTaskStore::new(database.pool.clone());
    let timed_out = new_task(&store).await;
    store
        .start_audio_processing(timed_out.task_id)
        .await
        .unwrap();
    store
        .finish_workflow(
            timed_out.task_id,
            database::PipelineTaskOutcome::TimedOut,
            None,
            Some("States.Timeout"),
            Some("waiter expired"),
        )
        .await
        .unwrap();
    store
        .finish_workflow(
            timed_out.task_id,
            database::PipelineTaskOutcome::TimedOut,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let timeout_row: (String, String, Option<String>) = sqlx::query_as(
        "SELECT task.outcome::TEXT, audio.status::TEXT, audio.error_code FROM pipeline_tasks task \
         JOIN audio_processing_tasks audio USING (task_id) WHERE task.task_id = $1",
    )
    .bind(timed_out.task_id)
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert_eq!(
        timeout_row,
        (
            "TIMED_OUT".into(),
            "FAILED".into(),
            Some("States.Timeout".into())
        )
    );

    let failed = store
        .create_or_get(NewPipelineTask {
            tenant_id: "tenant-lifecycle",
            idempotency_key: "request-failure-callback",
            caller_reference: None,
            audio_s3_uris: &["s3://uploads/failure.wav".to_owned()],
        })
        .await
        .unwrap();
    store.start_audio_processing(failed.task_id).await.unwrap();
    store
        .complete_audio_processing(failed.task_id, "s3://artifacts/evaluations/failure.wav")
        .await
        .unwrap();
    store
        .start_asr(failed.task_id, "failure-token")
        .await
        .unwrap();
    assert_eq!(
        store
            .record_asr_failure("failure-token", Some("ASR_FAILED"), Some("details"))
            .await
            .unwrap(),
        failed.task_id
    );
    assert_eq!(
        store
            .record_asr_failure("failure-token", Some("ASR_FAILED"), Some("details"))
            .await
            .unwrap(),
        failed.task_id
    );
    assert!(matches!(
        store
            .record_asr_failure("failure-token", Some("ASR_FAILED"), Some("other details"))
            .await,
        Err(PipelineTaskError::TranscriptionConflict)
    ));
    store
        .finalize_asr_failure(failed.task_id, "failure-token")
        .await
        .unwrap();
    let failure_outcome: String =
        sqlx::query_scalar("SELECT outcome::TEXT FROM pipeline_tasks WHERE task_id = $1")
            .bind(failed.task_id)
            .fetch_one(&database.pool)
            .await
            .unwrap();
    assert_eq!(failure_outcome, "FAILED");
}

async fn new_task(store: &PipelineTaskStore) -> database::PipelineTask {
    store
        .create_or_get(NewPipelineTask {
            tenant_id: "tenant-lifecycle",
            idempotency_key: "request-lifecycle",
            caller_reference: None,
            audio_s3_uris: &["s3://uploads/audio.wav".to_owned()],
        })
        .await
        .unwrap()
}

async fn complete_asr(store: &PipelineTaskStore, task_id: i32, token: &str) {
    store.start_audio_processing(task_id).await.unwrap();
    store
        .complete_audio_processing(task_id, "s3://artifacts/evaluations/audio.wav")
        .await
        .unwrap();
    store.start_asr(task_id, token).await.unwrap();
    store
        .complete_asr(task_id, token, "fc-123", "hello world")
        .await
        .unwrap();
}

fn moderation_result() -> ModerationResult {
    ModerationResult {
        sexual: 0.02,
        hate_or_discrimination: 0.15,
        harassment_or_abuse: 0.08,
        violence_or_threats: 0.01,
        asking_for_pii: 0.42,
    }
}
