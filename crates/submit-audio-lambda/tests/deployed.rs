use std::collections::HashMap;
use std::env;
use std::time::Duration;

use reqwest::header::{HeaderName, HeaderValue};
use serde_json::{Value, json};
use uuid::Uuid;

const SUBMIT_REVIEW_PATH: &str = "/audio.review.v1.AudioReviewService/SubmitReview";

#[tokio::test]
#[ignore = "requires AUDIO_MODERATION_API_ENDPOINT and a deployed AWS environment"]
async fn submits_uploads_and_replays_against_aws() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = env::var("AUDIO_MODERATION_API_ENDPOINT")?;
    let submit_url = format!("{}{}", endpoint.trim_end_matches('/'), SUBMIT_REVIEW_PATH);
    let idempotency_key = format!("deployed-test-{}", Uuid::new_v4());
    let client = reqwest::Client::new();

    let submitted = submit(&client, &submit_url, &idempotency_key).await?;
    assert_eq!(
        submitted["status"],
        Value::String("REVIEW_JOB_STATUS_AWAITING_UPLOAD".to_owned())
    );

    let task_id = submitted["taskId"]
        .as_str()
        .expect("submit response must contain taskId");
    let upload_url = submitted["uploadUrl"]
        .as_str()
        .expect("submit response must contain uploadUrl");
    let upload_headers: HashMap<String, String> =
        serde_json::from_value(submitted["uploadHeaders"].clone())?;

    let mut upload = client.put(upload_url).body(minimal_wav());
    for (name, value) in upload_headers {
        upload = upload.header(
            HeaderName::from_bytes(name.as_bytes())?,
            HeaderValue::from_str(&value)?,
        );
    }
    let upload_response = upload.send().await?;
    assert!(
        upload_response.status().is_success(),
        "S3 upload failed with {}: {}",
        upload_response.status(),
        upload_response.text().await?
    );

    let replayed = wait_for_status(
        &client,
        &submit_url,
        &idempotency_key,
        "REVIEW_JOB_STATUS_PENDING_PROCESSING",
    )
    .await?;
    assert_eq!(replayed["taskId"], task_id);

    Ok(())
}

async fn wait_for_status(
    client: &reqwest::Client,
    url: &str,
    idempotency_key: &str,
    expected_status: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    for _ in 0..20 {
        let response = submit(client, url, idempotency_key).await?;
        if response["status"] == expected_status {
            return Ok(response);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    Err(format!("task did not reach {expected_status} within 10 seconds").into())
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
