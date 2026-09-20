use std::collections::HashMap;
use std::env;
use std::io::Error as IoError;
use std::path::Path;
use std::time::Duration;

use aws_config::BehaviorVersion;
use aws_sdk_s3::Client as S3Client;
use aws_sdk_sfn::Client as SfnClient;
use reqwest::header::{HeaderName, HeaderValue};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

const SUBMIT_REVIEW_PATH: &str = "/audio.review.v1.AudioReviewService/SubmitReview";
const GET_REVIEW_PATH: &str = "/audio.review.v1.AudioReviewService/GetReview";
const GET_EVALUATION_PATH: &str = "/audio.moderation.v1.AudioModerationService/GetEvaluation";
const CREATE_TICKET_PATH: &str =
    "/audio.moderation.v1.AudioModerationService/CreateTaskEventsTicket";
const DISPATCH_TIMEOUT: Duration = Duration::from_secs(60);
const DUPLICATE_NOTIFICATION_WINDOW: Duration = Duration::from_secs(45);
const SUCCESSFUL_LIFECYCLE_EVENTS: [&str; 8] = [
    "EVALUATION_ACCEPTED",
    "AUDIO_PROCESSING_STARTED",
    "AUDIO_PROCESSING_FINISHED",
    "ASR_STARTED",
    "ASR_FINISHED",
    "MODERATION_PROCESSING_STARTED",
    "MODERATION_PROCESSING_FINISHED",
    "SUCCEEDED",
];

#[derive(Debug, sqlx::FromRow)]
struct StoredPipelineTask {
    task_id: i32,
    evaluation_id: String,
    caller_reference: Option<String>,
    execution_arn: Option<String>,
    attempt_count: i32,
    active_lease: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct ProcessingReportRow {
    outcome: Option<String>,
    audio_status: String,
    stitched_audio_s3_uri: Option<String>,
    transcription_status: String,
    transcription_task_id: Option<String>,
    transcription: Option<String>,
    moderation_status: String,
    moderation_task_id: Option<String>,
    sexual: Option<f64>,
    hate_or_discrimination: Option<f64>,
    harassment_or_abuse: Option<f64>,
    violence_or_threats: Option<f64>,
    asking_for_pii: Option<f64>,
}

#[tokio::test]
#[ignore = "requires AUDIO_MODERATION_API_ENDPOINT and a deployed AWS environment"]
async fn submits_review_against_aws() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = env::var("AUDIO_MODERATION_API_ENDPOINT")?;
    let submit_url = format!("{}{}", endpoint.trim_end_matches('/'), SUBMIT_REVIEW_PATH);
    let idempotency_key = format!("deployed-test-{}", Uuid::new_v4());
    let client = reqwest::Client::new();

    let submitted = submit(&client, &submit_url, &idempotency_key, "audio/wav").await?;
    assert_eq!(
        submitted["status"],
        Value::String("REVIEW_JOB_STATUS_AWAITING_UPLOAD".to_owned())
    );

    assert!(
        submitted["taskId"]
            .as_str()
            .is_some_and(|task_id| !task_id.is_empty())
    );
    assert!(
        submitted["uploadUrl"]
            .as_str()
            .is_some_and(|upload_url| !upload_url.is_empty())
    );
    assert!(submitted["uploadHeaders"].is_object());
    assert_eq!(submitted["reviewId"], submitted["taskId"]);
    let access_token = submitted["accessToken"]
        .as_str()
        .expect("submit response must contain accessToken");
    assert!(access_token.starts_with("review_v1."));
    assert!(submitted["accessExpiresAt"].is_string());
    let review = get_review(
        &client,
        &endpoint,
        submitted["taskId"].as_str().unwrap(),
        access_token,
    )
    .await?;
    assert_eq!(review["status"], "REVIEW_JOB_STATUS_AWAITING_UPLOAD");
    assert!(review.get("evaluationId").is_none());

    Ok(())
}

#[tokio::test]
#[ignore = "requires AUDIO_MODERATION_API_ENDPOINT and a deployed AWS environment"]
async fn starts_uploaded_review_evaluation_against_aws() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = env::var("AUDIO_MODERATION_API_ENDPOINT")?;
    let tenant_id = required_env("AUDIO_MODERATION_TENANT_ID")?;
    let uploads_bucket = required_env("AUDIO_MODERATION_UPLOADS_BUCKET")?;
    let artifacts_bucket = required_env("AUDIO_MODERATION_ARTIFACTS_BUCKET")?;
    let state_machine_arn = required_env("AUDIO_MODERATION_STATE_MACHINE_ARN")?;
    let database_url = required_env("DATABASE_URL")?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await?;
    let submit_url = format!("{}{}", endpoint.trim_end_matches('/'), SUBMIT_REVIEW_PATH);
    let idempotency_key = format!("deployed-test-{}", Uuid::new_v4());
    let client = reqwest::Client::new();

    let submitted = submit(&client, &submit_url, &idempotency_key, "audio/wav").await?;
    let task_id = submitted["taskId"]
        .as_str()
        .expect("submit response must contain taskId")
        .to_owned();
    let upload_url = submitted["uploadUrl"]
        .as_str()
        .expect("submit response must contain uploadUrl");
    let upload_headers: HashMap<String, String> =
        serde_json::from_value(submitted["uploadHeaders"].clone())?;
    let audio = minimal_wav();

    put_presigned(&client, upload_url, &upload_headers, audio.clone()).await?;

    let pipeline_key = format!("review-upload:{task_id}");
    let expected_caller_reference = format!("review-job:{task_id}");
    let task = wait_for_dispatched_task(&pool, &tenant_id, &pipeline_key).await?;
    let input_file_path = review_input_path(&pool, &tenant_id, &task_id).await?;
    let uploaded_uri = format!("s3://{uploads_bucket}/{input_file_path}");
    let inputs = stored_inputs(&pool, task.task_id).await?;

    assert_eq!(
        task.caller_reference.as_deref(),
        Some(expected_caller_reference.as_str())
    );
    assert_eq!(inputs, vec![uploaded_uri.clone()]);
    assert_eq!(
        task.attempt_count, 1,
        "upload should have one dispatch attempt"
    );
    assert!(
        !task.active_lease,
        "recorded execution must release its dispatch lease"
    );
    let execution_arn = task
        .execution_arn
        .as_deref()
        .expect("dispatched review task must persist an execution ARN")
        .to_owned();

    assert_execution(
        &state_machine_arn,
        &artifacts_bucket,
        &task,
        &uploaded_uri,
        &execution_arn,
    )
    .await?;

    // A second PUT to the same still-valid presigned URL can produce another S3
    // ObjectCreated notification. Keep checking for a full delivery window for
    // duplicate persisted effects; the test cannot establish notification delivery.
    put_presigned(&client, upload_url, &upload_headers, audio).await?;
    assert_duplicate_notification_is_idempotent(
        &pool,
        &tenant_id,
        &pipeline_key,
        &task,
        &inputs,
        &execution_arn,
    )
    .await?;

    Ok(())
}

#[tokio::test]
#[ignore = "requires a deployed AWS environment, compatible model endpoints, and a real audio file"]
async fn completes_uploaded_review_evaluation_against_aws() -> Result<(), Box<dyn std::error::Error>>
{
    let endpoint = required_env("AUDIO_MODERATION_API_ENDPOINT")?;
    let tenant_id = required_env("AUDIO_MODERATION_TENANT_ID")?;
    let uploads_bucket = required_env("AUDIO_MODERATION_UPLOADS_BUCKET")?;
    let artifacts_bucket = required_env("AUDIO_MODERATION_ARTIFACTS_BUCKET")?;
    let state_machine_arn = required_env("AUDIO_MODERATION_STATE_MACHINE_ARN")?;
    let audio_path = required_env("AUDIO_MODERATION_TEST_AUDIO_FILE")?;
    let audio = std::fs::read(&audio_path)?;
    if audio.is_empty() {
        return Err(IoError::other("AUDIO_MODERATION_TEST_AUDIO_FILE must not be empty").into());
    }

    let database_url = required_env("DATABASE_URL")?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await?;
    let submit_url = format!("{}{}", endpoint.trim_end_matches('/'), SUBMIT_REVIEW_PATH);
    let idempotency_key = format!("deployed-e2e-{}", Uuid::new_v4());
    let client = reqwest::Client::new();
    let content_type = audio_content_type(Path::new(&audio_path));

    let submitted = submit(&client, &submit_url, &idempotency_key, content_type).await?;
    let replayed = submit(&client, &submit_url, &idempotency_key, content_type).await?;
    let task_id = submitted["taskId"]
        .as_str()
        .ok_or_else(|| IoError::other("submit response must contain taskId"))?
        .to_owned();
    assert_eq!(replayed["taskId"], task_id);
    assert_eq!(replayed["status"], submitted["status"]);

    let source_key = review_input_path(&pool, &tenant_id, &task_id).await?;
    let source_uri = format!("s3://{uploads_bucket}/{source_key}");
    let result = async {
        let upload_url = submitted["uploadUrl"]
            .as_str()
            .ok_or_else(|| IoError::other("submit response must contain uploadUrl"))?;
        let upload_headers: HashMap<String, String> =
            serde_json::from_value(submitted["uploadHeaders"].clone())?;
        put_presigned(&client, upload_url, &upload_headers, audio).await?;

        let pipeline_key = format!("review-upload:{task_id}");
        let task = wait_for_dispatched_task(&pool, &tenant_id, &pipeline_key).await?;
        let review_token = submitted["accessToken"]
            .as_str()
            .ok_or_else(|| IoError::other("submit response must contain accessToken"))?;
        let review = get_review(&client, &endpoint, &task_id, review_token).await?;
        assert_eq!(review["evaluationId"], task.evaluation_id);
        let evaluation = authorized_rpc(
            &client,
            &endpoint,
            GET_EVALUATION_PATH,
            review_token,
            json!({ "evaluationId": task.evaluation_id.clone() }),
        )
        .await?;
        assert_eq!(evaluation["evaluation"]["evaluationId"], task.evaluation_id);
        let ticket = authorized_rpc(
            &client,
            &endpoint,
            CREATE_TICKET_PATH,
            review_token,
            json!({ "evaluationId": task.evaluation_id.clone() }),
        )
        .await?;
        assert!(
            ticket["ticket"]
                .as_str()
                .is_some_and(|value| value.starts_with("wst_v1."))
        );
        let expected_caller_reference = format!("review-job:{task_id}");
        assert_eq!(
            task.caller_reference.as_deref(),
            Some(expected_caller_reference.as_str())
        );
        assert_eq!(
            stored_inputs(&pool, task.task_id).await?,
            vec![source_uri.clone()]
        );
        assert_execution(
            &state_machine_arn,
            &artifacts_bucket,
            &task,
            &source_uri,
            task.execution_arn
                .as_deref()
                .ok_or_else(|| IoError::other("dispatched task has no execution ARN"))?,
        )
        .await?;
        wait_for_terminal_success(&pool, &artifacts_bucket, &task).await
    }
    .await;

    match result {
        Ok(()) => {
            if let Err(error) = delete_source(&uploads_bucket, &source_key).await {
                println!("retaining source after cleanup failure: {source_uri}");
                Err(error)
            } else {
                Ok(())
            }
        }
        Err(error) => {
            println!("retaining source after evaluation failure: {source_uri}");
            Err(error)
        }
    }
}

async fn wait_for_terminal_success(
    pool: &PgPool,
    artifacts_bucket: &str,
    task: &StoredPipelineTask,
) -> Result<(), Box<dyn std::error::Error>> {
    let execution_arn = task
        .execution_arn
        .as_deref()
        .ok_or_else(|| IoError::other("dispatched task has no execution ARN"))?;
    let sdk_config = aws_config::defaults(BehaviorVersion::latest())
        .profile_name("admin")
        .load()
        .await;
    let sfn = SfnClient::new(&sdk_config);
    let deadline = tokio::time::Instant::now() + workflow_timeout();

    loop {
        let execution = sfn
            .describe_execution()
            .execution_arn(execution_arn)
            .send()
            .await?;
        match execution.status().as_str() {
            "SUCCEEDED" => {
                let report = processing_report(pool, task.task_id).await?;
                print_processing_report(task.task_id, execution_arn, "SUCCEEDED", &report)?;
                assert_terminal_persistence(artifacts_bucket, task.task_id, &report).await?;
                assert_durable_lifecycle(pool, task.task_id).await?;
                return Ok(());
            }
            "FAILED" | "TIMED_OUT" | "ABORTED" => {
                let report = processing_report(pool, task.task_id).await?;
                print_processing_report(
                    task.task_id,
                    execution_arn,
                    execution.status().as_str(),
                    &report,
                )?;
                return Err(IoError::other(format!(
                    "workflow {execution_arn} ended as {}",
                    execution.status().as_str()
                ))
                .into());
            }
            _ if tokio::time::Instant::now() >= deadline => {
                let report = processing_report(pool, task.task_id).await?;
                print_processing_report(task.task_id, execution_arn, "WAIT_TIMEOUT", &report)?;
                return Err(IoError::other(format!(
                    "workflow {execution_arn} did not complete within {} seconds",
                    workflow_timeout().as_secs()
                ))
                .into());
            }
            _ => tokio::time::sleep(Duration::from_secs(2)).await,
        }
    }
}

async fn processing_report(
    pool: &PgPool,
    task_id: i32,
) -> Result<ProcessingReportRow, sqlx::Error> {
    sqlx::query_as(
        r#"
            SELECT task.outcome::TEXT AS outcome,
                audio.status::TEXT AS audio_status,
                audio.stitched_audio_s3_uri,
                transcription.status::TEXT AS transcription_status,
                transcription.external_task_id AS transcription_task_id,
                transcription.transcript AS transcription,
                moderation.status::TEXT AS moderation_status,
                moderation.external_task_id AS moderation_task_id,
                moderation.sexual,
                moderation.hate_or_discrimination,
                moderation.harassment_or_abuse,
                moderation.violence_or_threats,
                moderation.asking_for_pii
            FROM pipeline_tasks task
            JOIN audio_processing_tasks audio USING (task_id)
            JOIN transcription_tasks transcription USING (task_id)
            JOIN moderation_tasks moderation USING (task_id)
            WHERE task.task_id = $1
        "#,
    )
    .bind(task_id)
    .fetch_one(pool)
    .await
}

fn print_processing_report(
    task_id: i32,
    execution_arn: &str,
    execution_status: &str,
    report: &ProcessingReportRow,
) -> Result<(), serde_json::Error> {
    let moderation_scores_persisted = [
        report.sexual,
        report.hate_or_discrimination,
        report.harassment_or_abuse,
        report.violence_or_threats,
        report.asking_for_pii,
    ]
    .iter()
    .all(Option::is_some);
    println!(
        "Processing result:\n{}",
        serde_json::to_string_pretty(&json!({
            "pipelineTaskId": task_id,
            "execution": { "arn": execution_arn, "status": execution_status },
            "pipelineOutcome": report.outcome,
            "artifact": report.stitched_audio_s3_uri,
            "audioProcessingStatus": report.audio_status,
            "transcription": {
                "status": report.transcription_status,
                "taskId": report.transcription_task_id,
                "persisted": report.transcription.is_some(),
            },
            "moderation": {
                "status": report.moderation_status,
                "taskId": report.moderation_task_id,
                "scoresPersisted": moderation_scores_persisted,
            },
        }))?
    );
    Ok(())
}

async fn assert_terminal_persistence(
    artifacts_bucket: &str,
    task_id: i32,
    report: &ProcessingReportRow,
) -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(report.outcome.as_deref(), Some("SUCCEEDED"));
    assert_eq!(report.audio_status, "COMPLETED");
    assert_eq!(report.transcription_status, "COMPLETED");
    assert_eq!(report.moderation_status, "COMPLETED");
    assert!(
        report
            .transcription_task_id
            .as_deref()
            .is_some_and(|id| !id.is_empty())
    );
    assert!(
        report.transcription.is_some(),
        "completed transcription must persist a result"
    );
    assert!(
        report
            .moderation_task_id
            .as_deref()
            .is_some_and(|id| !id.is_empty())
    );
    for score in [
        report.sexual,
        report.hate_or_discrimination,
        report.harassment_or_abuse,
        report.violence_or_threats,
        report.asking_for_pii,
    ] {
        assert!(score.is_some_and(|score| (0.0..=1.0).contains(&score)));
    }

    let expected_artifact = format!("s3://{artifacts_bucket}/evaluations/{task_id}.wav");
    assert_eq!(
        report.stitched_audio_s3_uri.as_deref(),
        Some(expected_artifact.as_str())
    );
    let sdk_config = aws_config::defaults(BehaviorVersion::latest())
        .profile_name("admin")
        .load()
        .await;
    S3Client::new(&sdk_config)
        .head_object()
        .bucket(artifacts_bucket)
        .key(format!("evaluations/{task_id}.wav"))
        .send()
        .await?;
    Ok(())
}

async fn assert_durable_lifecycle(
    pool: &PgPool,
    task_id: i32,
) -> Result<(), Box<dyn std::error::Error>> {
    let events: Vec<String> = sqlx::query_scalar(
        "SELECT event_name FROM pipeline_task_events WHERE task_id = $1 ORDER BY event_id",
    )
    .bind(task_id)
    .fetch_all(pool)
    .await?;
    let missing: Vec<_> = SUCCESSFUL_LIFECYCLE_EVENTS
        .iter()
        .copied()
        .filter(|expected| !events.iter().any(|event| event == expected))
        .collect();
    assert!(
        missing.is_empty(),
        "durable lifecycle history missed {missing:?}; observed {events:?}"
    );
    Ok(())
}

async fn delete_source(
    uploads_bucket: &str,
    source_key: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let sdk_config = aws_config::defaults(BehaviorVersion::latest())
        .profile_name("admin")
        .load()
        .await;
    S3Client::new(&sdk_config)
        .delete_object()
        .bucket(uploads_bucket)
        .key(source_key)
        .send()
        .await?;
    Ok(())
}

async fn put_presigned(
    client: &reqwest::Client,
    upload_url: &str,
    upload_headers: &HashMap<String, String>,
    body: Vec<u8>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut upload = client.put(upload_url).body(body);
    for (name, value) in upload_headers {
        upload = upload.header(
            HeaderName::from_bytes(name.as_bytes())?,
            HeaderValue::from_str(value)?,
        );
    }
    let response = upload.send().await?;
    let status = response.status();
    let body = response.text().await?;
    assert!(
        status.is_success(),
        "S3 upload failed with {status}: {body}"
    );

    Ok(())
}

async fn wait_for_dispatched_task(
    pool: &PgPool,
    tenant_id: &str,
    pipeline_key: &str,
) -> Result<StoredPipelineTask, Box<dyn std::error::Error>> {
    let deadline = tokio::time::Instant::now() + DISPATCH_TIMEOUT;

    loop {
        let tasks = matching_tasks(pool, tenant_id, pipeline_key).await?;
        let last_state = match tasks.as_slice() {
            [task] => {
                let inputs = stored_inputs(pool, task.task_id).await?;
                let state = format!("task={task:?}, inputs={inputs:?}");
                if task.execution_arn.is_some() && !task.active_lease {
                    return Ok(StoredPipelineTask {
                        task_id: task.task_id,
                        evaluation_id: task.evaluation_id.clone(),
                        caller_reference: task.caller_reference.clone(),
                        execution_arn: task.execution_arn.clone(),
                        attempt_count: task.attempt_count,
                        active_lease: task.active_lease,
                    });
                }
                state
            }
            [] => "no matching pipeline task observed".to_owned(),
            _ => {
                return Err(IoError::other(format!(
                    "review upload created {} matching pipeline tasks for {pipeline_key}; tasks={tasks:?}",
                    tasks.len()
                ))
                .into());
            }
        };

        if tokio::time::Instant::now() >= deadline {
            return Err(IoError::other(format!(
                "review upload did not create and dispatch pipeline task {pipeline_key} within {:?}; last state: {last_state}",
                DISPATCH_TIMEOUT
            ))
            .into());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn assert_duplicate_notification_is_idempotent(
    pool: &PgPool,
    tenant_id: &str,
    pipeline_key: &str,
    expected_task: &StoredPipelineTask,
    expected_inputs: &[String],
    expected_execution_arn: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = tokio::time::Instant::now() + DUPLICATE_NOTIFICATION_WINDOW;

    loop {
        let tasks = matching_tasks(pool, tenant_id, pipeline_key).await?;
        if tasks.len() != 1 {
            return Err(IoError::other(format!(
                "duplicate S3 notification created {} matching tasks for {pipeline_key}; matching tasks={tasks:?}",
                tasks.len(),
            ))
            .into());
        }
        let task = &tasks[0];
        let inputs = stored_inputs(pool, task.task_id).await?;
        let last_state = format!("task={task:?}, inputs={inputs:?}");
        if task.task_id != expected_task.task_id
            || task.evaluation_id != expected_task.evaluation_id
            || task.caller_reference != expected_task.caller_reference
            || task.execution_arn.as_deref() != Some(expected_execution_arn)
            || task.attempt_count != expected_task.attempt_count
            || task.active_lease != expected_task.active_lease
            || inputs != expected_inputs
        {
            return Err(IoError::other(format!(
                "duplicate S3 notification changed review dispatch state; expected task={expected_task:?}, inputs={expected_inputs:?}; last state: {last_state}"
            ))
            .into());
        }

        if tokio::time::Instant::now() >= deadline {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn matching_tasks(
    pool: &PgPool,
    tenant_id: &str,
    pipeline_key: &str,
) -> Result<Vec<StoredPipelineTask>, sqlx::Error> {
    sqlx::query_as(
        r#"
            SELECT task_id, evaluation_id::TEXT AS evaluation_id, caller_reference,
                execution_arn, attempt_count, dispatch_started_at IS NOT NULL AS active_lease
            FROM pipeline_tasks
            WHERE tenant_id = $1 AND idempotency_key = $2
        "#,
    )
    .bind(tenant_id)
    .bind(pipeline_key)
    .fetch_all(pool)
    .await
}

async fn stored_inputs(pool: &PgPool, task_id: i32) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT audio_s3_uri FROM pipeline_task_inputs WHERE task_id = $1 ORDER BY sequence",
    )
    .bind(task_id)
    .fetch_all(pool)
    .await
}

async fn review_input_path(
    pool: &PgPool,
    tenant_id: &str,
    review_job_id: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let review_job_id = review_job_id.parse::<i32>()?;
    sqlx::query_scalar(
        "SELECT input_file_path FROM review_jobs WHERE job_id = $1 AND tenant_id = $2",
    )
    .bind(review_job_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| IoError::other(format!("review job {review_job_id} was not persisted")))
    .map_err(Into::into)
}

async fn assert_execution(
    state_machine_arn: &str,
    artifacts_bucket: &str,
    task: &StoredPipelineTask,
    uploaded_uri: &str,
    execution_arn: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let sdk_config = aws_config::defaults(BehaviorVersion::latest())
        .profile_name("admin")
        .load()
        .await;
    let execution = SfnClient::new(&sdk_config)
        .describe_execution()
        .execution_arn(execution_arn)
        .send()
        .await?;

    assert_eq!(execution.execution_arn(), execution_arn);
    let expected_name = format!("evaluation-{}", task.evaluation_id);
    assert_eq!(execution.name(), Some(expected_name.as_str()));
    assert_eq!(execution.state_machine_arn(), state_machine_arn);
    assert!(
        !execution.status().as_str().is_empty(),
        "execution has no Step Functions status"
    );
    let input: Value = serde_json::from_str(
        execution
            .input()
            .ok_or_else(|| IoError::other("DescribeExecution response has no input"))?,
    )?;
    assert_eq!(input["jobId"], task.task_id.to_string());
    assert_eq!(
        input["files"],
        json!([{ "s3Uri": uploaded_uri, "sequence": 0 }])
    );
    assert_eq!(
        input["outputS3Uri"],
        format!("s3://{artifacts_bucket}/evaluations/{}.wav", task.task_id)
    );

    Ok(())
}

fn required_env(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    let value = env::var(name)?;
    if value.trim().is_empty() {
        return Err(IoError::other(format!("{name} must not be empty")).into());
    }
    Ok(value)
}

async fn submit(
    client: &reqwest::Client,
    url: &str,
    idempotency_key: &str,
    content_type: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    let response = client
        .post(url)
        .header("content-type", "application/json")
        .header("connect-protocol-version", "1")
        .header("idempotency-key", idempotency_key)
        .json(&json!({ "contentType": content_type }))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    assert!(status.is_success(), "submit failed with {status}: {body}");
    Ok(serde_json::from_str(&body)?)
}

async fn get_review(
    client: &reqwest::Client,
    endpoint: &str,
    review_id: &str,
    access_token: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    let response = client
        .post(format!(
            "{}{}",
            endpoint.trim_end_matches('/'),
            GET_REVIEW_PATH
        ))
        .header("content-type", "application/json")
        .header("connect-protocol-version", "1")
        .bearer_auth(access_token)
        .json(&json!({ "reviewId": review_id }))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    assert!(
        status.is_success(),
        "get review failed with {status}: {body}"
    );
    Ok(serde_json::from_str::<Value>(&body)?["review"].clone())
}

async fn authorized_rpc(
    client: &reqwest::Client,
    endpoint: &str,
    path: &str,
    access_token: &str,
    request: Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let response = client
        .post(format!("{}{}", endpoint.trim_end_matches('/'), path))
        .header("content-type", "application/json")
        .header("connect-protocol-version", "1")
        .bearer_auth(access_token)
        .json(&request)
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    assert!(
        status.is_success(),
        "RPC {path} failed with {status}: {body}"
    );
    Ok(serde_json::from_str(&body)?)
}

fn audio_content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("mp3") => "audio/mpeg",
        Some("m4a") | Some("mp4") => "audio/mp4",
        Some("ogg") => "audio/ogg",
        Some("flac") => "audio/flac",
        _ => "audio/wav",
    }
}

fn workflow_timeout() -> Duration {
    env::var("AUDIO_MODERATION_WORKFLOW_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(600))
}

fn minimal_wav() -> Vec<u8> {
    let mut wav = b"RIFF".to_vec();
    wav.extend_from_slice(&36_u32.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&8_000_u32.to_le_bytes());
    wav.extend_from_slice(&16_000_u32.to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&0_u32.to_le_bytes());
    wav
}
