use std::env;
use std::error::Error;
use std::io::{Error as IoError, ErrorKind};

use audio_processing_lambda::AudioSource;
use aws_config::BehaviorVersion;
use aws_sdk_lambda::primitives::Blob;
use aws_sdk_s3::primitives::ByteStream;
use database::{NewPipelineTask, PipelineStepStatus, PipelineTaskStore};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

type TestError = Box<dyn Error + Send + Sync>;

#[tokio::test]
#[ignore = "requires a deployed audio-processing Lambda, S3 buckets, and PostgreSQL"]
async fn converts_audio_against_aws() -> Result<(), TestError> {
    let tenant_id = required_env("AUDIO_MODERATION_TENANT_ID")?;
    let uploads_bucket = required_env("AUDIO_MODERATION_UPLOADS_BUCKET")?;
    let artifacts_bucket = required_env("AUDIO_MODERATION_ARTIFACTS_BUCKET")?;
    let function_name = required_env("AUDIO_MODERATION_AUDIO_PROCESSING_FUNCTION_NAME")?;
    let audio_file = required_env("AUDIO_MODERATION_AUDIO_FILE")?;
    let database_url = required_env("DATABASE_URL")?;
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect(&database_url)
        .await?;
    let store = PipelineTaskStore::new(pool);
    let config = aws_config::defaults(BehaviorVersion::latest())
        .profile_name("admin")
        .load()
        .await;
    let s3 = aws_sdk_s3::Client::new(&config);
    let lambda = aws_sdk_lambda::Client::new(&config);

    let run_id = Uuid::new_v4();
    let input_key = format!("reviews/integration-tests/{run_id}/audio.wav");
    let output_key = format!("evaluations/integration-tests/{run_id}/stitched.wav");
    let input_uri = format!("s3://{uploads_bucket}/{input_key}");
    let output_uri = format!("s3://{artifacts_bucket}/{output_key}");
    let idempotency_key = format!("audio-conversion-{run_id}");
    let mut task_id = None;

    let result: Result<(), TestError> = async {
        s3.put_object()
            .bucket(&uploads_bucket)
            .key(&input_key)
            .body(ByteStream::from_path(audio_file).await?)
            .send()
            .await?;

        let task = store
            .create_or_get(NewPipelineTask {
                tenant_id: &tenant_id,
                idempotency_key: &idempotency_key,
                caller_reference: None,
                audio_s3_uris: &[input_uri.clone()],
            })
            .await?;
        if !task.created {
            return Err(invalid_data(
                "audio-conversion fixture was not newly created",
            ));
        }
        task_id = Some(task.task_id);

        let payload = json!({
            "jobId": task.task_id.to_string(),
            "files": [AudioSource { s3_uri: input_uri.clone(), sequence: 0 }],
            "outputS3Uri": output_uri,
        });
        let invocation = lambda
            .invoke()
            .function_name(&function_name)
            .payload(Blob::new(serde_json::to_vec(&payload)?))
            .send()
            .await?;
        if invocation.status_code() != 200 || invocation.function_error().is_some() {
            return Err(invalid_data(&format!(
                "audio-processing Lambda invocation failed (status {}, function error {:?})",
                invocation.status_code(),
                invocation.function_error()
            )));
        }
        let response: Value = serde_json::from_slice(
            invocation
                .payload()
                .ok_or_else(|| invalid_data("audio-processing Lambda returned no payload"))?
                .as_ref(),
        )?;
        if response["jobId"] != task.task_id.to_string() || response["stitchedS3Uri"] != output_uri
        {
            return Err(invalid_data(
                "audio-processing Lambda returned unexpected output",
            ));
        }

        s3.head_object()
            .bucket(&artifacts_bucket)
            .key(&output_key)
            .send()
            .await?;
        let details = store
            .get_details(Uuid::parse_str(&task.evaluation_id)?, &tenant_id)
            .await?;
        if details.audio_processing.status != PipelineStepStatus::Completed
            || details.audio_processing.stitched_audio_s3_uri.as_deref() != Some(&output_uri)
        {
            return Err(invalid_data("audio-processing result was not persisted"));
        }
        Ok(())
    }
    .await;

    // Direct Lambda invocation is synchronous; nothing else uses these fixtures.
    // Always attempt every cleanup action, even if an earlier action fails.
    let mut cleanup_error = None;
    for (bucket, key) in [
        (&uploads_bucket, &input_key),
        (&artifacts_bucket, &output_key),
    ] {
        if let Err(error) = s3.delete_object().bucket(bucket).key(key).send().await {
            eprintln!("Could not remove audio-conversion fixture s3://{bucket}/{key}: {error}");
            cleanup_error.get_or_insert_with(|| invalid_data("S3 fixture cleanup failed"));
        }
    }
    if let Some(task_id) = task_id {
        match store
            .delete_fixture(task_id, &tenant_id, &idempotency_key)
            .await
        {
            Ok(true) => {}
            Ok(false) => {
                eprintln!("Could not remove audio-conversion pipeline task: {task_id}");
                cleanup_error.get_or_insert_with(|| invalid_data("pipeline task cleanup failed"));
            }
            Err(error) => {
                eprintln!("Could not remove audio-conversion pipeline task {task_id}: {error}");
                cleanup_error.get_or_insert_with(|| invalid_data("pipeline task cleanup failed"));
            }
        }
    }
    result?;
    if let Some(error) = cleanup_error {
        return Err(error);
    }
    Ok(())
}

fn required_env(name: &str) -> Result<String, TestError> {
    env::var(name)
        .map_err(|_| IoError::new(ErrorKind::InvalidInput, format!("{name} is required")).into())
}

fn invalid_data(message: &str) -> TestError {
    IoError::new(ErrorKind::InvalidData, message).into()
}
