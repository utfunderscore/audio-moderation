use std::collections::HashMap;
use std::env;
use std::io::Error as IoError;
use std::time::Duration;

use aws_config::BehaviorVersion;
use aws_sdk_sfn::Client as SfnClient;
use reqwest::header::{HeaderName, HeaderValue};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

const SUBMIT_REVIEW_PATH: &str = "/audio.review.v1.AudioReviewService/SubmitReview";
const GET_REVIEW_PATH: &str = "/audio.review.v1.AudioReviewService/GetReview";
const DISPATCH_TIMEOUT: Duration = Duration::from_secs(60);
const DUPLICATE_NOTIFICATION_WINDOW: Duration = Duration::from_secs(45);

#[derive(Debug, sqlx::FromRow)]
struct StoredPipelineTask {
    task_id: i32,
    evaluation_id: String,
    review_job_id: Option<i32>,
    caller_reference: Option<String>,
    execution_arn: Option<String>,
    attempt_count: i32,
    active_lease: bool,
}

#[tokio::test]
#[ignore = "requires AUDIO_MODERATION_API_ENDPOINT and a deployed AWS environment"]
async fn submits_review_against_aws() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = env::var("AUDIO_MODERATION_API_ENDPOINT")?;
    let submit_url = format!("{}{}", endpoint.trim_end_matches('/'), SUBMIT_REVIEW_PATH);
    let idempotency_key = format!("deployed-test-{}", Uuid::new_v4());
    let client = reqwest::Client::new();

    let submitted = submit(&client, &submit_url, &idempotency_key).await?;
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
    assert!(
        submitted["accessToken"]
            .as_str()
            .is_some_and(|token| token.starts_with("review_v1."))
    );
    let review = get_review(
        &client,
        &endpoint,
        submitted["reviewId"].as_str().unwrap(),
        submitted["accessToken"].as_str().unwrap(),
    )
    .await?;
    assert_eq!(review["status"], "REVIEW_JOB_STATUS_AWAITING_UPLOAD");
    assert_eq!(review["evaluationId"], submitted["evaluationId"]);

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

    let submitted = submit(&client, &submit_url, &idempotency_key).await?;
    let task_id = submitted["taskId"]
        .as_str()
        .expect("submit response must contain taskId")
        .to_owned();
    assert!(
        submitted["evaluationId"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
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
    assert_eq!(task.review_job_id, Some(task_id.parse()?));
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
        "GetReview failed with {status}: {body}"
    );
    Ok(serde_json::from_str::<Value>(&body)?["review"].clone())
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
                        review_job_id: task.review_job_id,
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
            SELECT task_id, evaluation_id::TEXT AS evaluation_id, review_job_id, caller_reference,
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
) -> Result<Value, Box<dyn std::error::Error>> {
    let response = client
        .post(url)
        .header("content-type", "application/json")
        .header("connect-protocol-version", "1")
        .header("idempotency-key", idempotency_key)
        .json(&json!({ "contentType": "audio/wav" }))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    assert!(status.is_success(), "submit failed with {status}: {body}");
    Ok(serde_json::from_str(&body)?)
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
