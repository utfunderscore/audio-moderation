use std::borrow::Cow;

use aws_lambda_events::event::s3::S3Event;
use aws_sdk_s3::Client as S3Client;
use database::ReviewJobStore;
use lambda_runtime::{Error, LambdaEvent};
use tracing::info;

#[derive(Clone)]
pub struct ConfirmUploadHandler {
    store: ReviewJobStore,
    s3_client: S3Client,
    uploads_bucket: String,
    tenant_id: String,
}

impl ConfirmUploadHandler {
    pub fn new(
        store: ReviewJobStore,
        s3_client: S3Client,
        uploads_bucket: String,
        tenant_id: String,
    ) -> Self {
        Self {
            store,
            s3_client,
            uploads_bucket,
            tenant_id,
        }
    }

    pub async fn handle(&self, event: LambdaEvent<S3Event>) -> Result<(), Error> {
        let request_id = event.context.request_id;
        let objects = validate_correct_bucket(&event.payload, &self.uploads_bucket)?;

        for object in objects {
            self.s3_client
                .head_object()
                .bucket(&object.bucket)
                .key(&object.key)
                .send()
                .await?;

            let job = self
                .store
                .mark_upload_complete(&object.key, &self.tenant_id)
                .await?;
            let Some(job) = job else {
                return Err(ConfirmUploadError::UnknownObject(object.key).into());
            };

            info!(
                requestId = request_id,
                jobId = %job.job_id,
                tenantId = job.tenant_id,
                bucket = object.bucket,
                objectKey = object.key,
                status = ?job.status,
                outcome = "upload_confirmed",
                "confirmed source audio upload"
            );
        }

        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
struct UploadedObject {
    bucket: String,
    key: String,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
enum ConfirmUploadError {
    #[error("S3 event did not contain any records")]
    EmptyEvent,
    #[error("S3 event record did not contain a bucket name")]
    MissingBucket,
    #[error("S3 event record did not contain an object key")]
    MissingKey,
    #[error("received upload event from unexpected bucket {0}")]
    UnexpectedBucket(String),
    #[error("S3 object key is not valid UTF-8 after decoding")]
    InvalidKeyEncoding,
    #[error("no review job exists for uploaded object {0}")]
    UnknownObject(String),
}

fn validate_correct_bucket(
    event: &S3Event,
    expected_bucket: &str,
) -> Result<Vec<UploadedObject>, ConfirmUploadError> {
    if event.records.is_empty() {
        return Err(ConfirmUploadError::EmptyEvent);
    }

    event
        .records
        .iter()
        .map(|record| {
            let bucket = record
                .s3
                .bucket
                .name
                .clone()
                .ok_or(ConfirmUploadError::MissingBucket)?;
            if bucket != expected_bucket {
                return Err(ConfirmUploadError::UnexpectedBucket(bucket));
            }

            let encoded_key = record
                .s3
                .object
                .key
                .as_deref()
                .ok_or(ConfirmUploadError::MissingKey)?;
            let key = decode_s3_key(encoded_key)?;

            Ok(UploadedObject { bucket, key })
        })
        .collect()
}

fn decode_s3_key(encoded_key: &str) -> Result<String, ConfirmUploadError> {
    let encoded_key = if encoded_key.contains('+') {
        Cow::Owned(encoded_key.replace('+', " "))
    } else {
        Cow::Borrowed(encoded_key)
    };

    urlencoding::decode(&encoded_key)
        .map(Cow::into_owned)
        .map_err(|_| ConfirmUploadError::InvalidKeyEncoding)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s3_event(bucket: &str, key: &str) -> S3Event {
        serde_json::from_value(serde_json::json!({
            "Records": [{
                "eventVersion": "2.1",
                "eventSource": "aws:s3",
                "awsRegion": "eu-west-2",
                "eventTime": "2026-09-04T10:00:00Z",
                "eventName": "ObjectCreated:Put",
                "userIdentity": { "principalId": "test" },
                "requestParameters": { "sourceIPAddress": "127.0.0.1" },
                "responseElements": {
                    "x-amz-request-id": "request-id",
                    "x-amz-id-2": "host-id"
                },
                "s3": {
                    "s3SchemaVersion": "1.0",
                    "configurationId": "confirm-upload",
                    "bucket": {
                        "name": bucket,
                        "ownerIdentity": { "principalId": "test" },
                        "arn": format!("arn:aws:s3:::{bucket}")
                    },
                    "object": {
                        "key": key,
                        "size": 44,
                        "eTag": "etag",
                        "sequencer": "001"
                    }
                }
            }]
        }))
        .unwrap()
    }

    #[test]
    fn extracts_and_decodes_uploaded_object() {
        let event = s3_event("uploads", "reviews%2Ftask+one%2Fsource");

        assert_eq!(
            validate_correct_bucket(&event, "uploads").unwrap(),
            vec![UploadedObject {
                bucket: "uploads".to_owned(),
                key: "reviews/task one/source".to_owned(),
            }]
        );
    }

    #[test]
    fn rejects_an_unexpected_bucket() {
        let error = validate_correct_bucket(&s3_event("other", "reviews/task/source"), "uploads")
            .unwrap_err();

        assert_eq!(
            error,
            ConfirmUploadError::UnexpectedBucket("other".to_owned())
        );
    }

    #[test]
    fn rejects_an_empty_event() {
        let event: S3Event = serde_json::from_value(serde_json::json!({ "Records": [] })).unwrap();

        assert_eq!(
            validate_correct_bucket(&event, "uploads").unwrap_err(),
            ConfirmUploadError::EmptyEvent
        );
    }
}
