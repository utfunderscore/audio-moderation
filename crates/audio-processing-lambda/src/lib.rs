use lambda_runtime::Error;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use tracing::info;

pub mod stitcher;

pub struct LocalAudioSegment {
    pub segment_id: u64,
    pub file: NamedTempFile,
    pub sequence: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioProcessingInput {
    pub job_id: String,
    pub files: Vec<AudioSource>,
    pub output_s3_uri: String,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioSource {
    pub s3_uri: String,
    pub sequence: u32,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioProcessingOutput {
    pub job_id: String,
    pub stitched_s3_uri: String,
}

pub async fn handle(input: AudioProcessingInput) -> Result<AudioProcessingOutput, Error> {
    let input = validate_input(input)?;
    info!(
        jobId = input.job_id,
        fileCount = input.files.len(),
        stitchedS3Uri = input.output_s3_uri,
        "received audio stitching request"
    );

    Ok(AudioProcessingOutput {
        job_id: input.job_id,
        stitched_s3_uri: input.output_s3_uri,
    })
}

fn validate_input(
    input: AudioProcessingInput,
) -> Result<AudioProcessingInput, AudioProcessingError> {
    if input.files.is_empty() {
        return Err(AudioProcessingError::NoFiles);
    }

    S3Location::parse(&input.output_s3_uri)?;
    for (index, file) in input.files.iter().enumerate() {
        S3Location::parse(&file.s3_uri)?;
        if input.files[..index]
            .iter()
            .any(|previous| previous.sequence == file.sequence)
        {
            return Err(AudioProcessingError::DuplicateSequence(file.sequence));
        }
    }

    Ok(input)
}

#[derive(Debug, PartialEq, Eq)]
struct S3Location {
    bucket: String,
    key: String,
}

impl S3Location {
    fn parse(uri: &str) -> Result<Self, AudioProcessingError> {
        let Some(location) = uri.strip_prefix("s3://") else {
            return Err(AudioProcessingError::InvalidS3Uri(uri.to_owned()));
        };
        let Some((bucket, key)) = location.split_once('/') else {
            return Err(AudioProcessingError::InvalidS3Uri(uri.to_owned()));
        };
        if bucket.is_empty() || key.is_empty() {
            return Err(AudioProcessingError::InvalidS3Uri(uri.to_owned()));
        }

        Ok(Self {
            bucket: bucket.to_owned(),
            key: key.to_owned(),
        })
    }
}

#[derive(Debug, thiserror::Error)]
enum AudioProcessingError {
    #[error("audio processing requires at least one file")]
    NoFiles,
    #[error("audio processing requires unique sequence values; {0} was repeated")]
    DuplicateSequence(u32),
    #[error("invalid S3 URI {0}")]
    InvalidS3Uri(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_multiple_ordered_s3_uris() {
        let input = AudioProcessingInput {
            job_id: "job-123".to_owned(),
            files: vec![
                AudioSource {
                    s3_uri: "s3://uploads/reviews/job-123/source-2.mp3".to_owned(),
                    sequence: 2,
                },
                AudioSource {
                    s3_uri: "s3://uploads/reviews/job-123/source-1.wav".to_owned(),
                    sequence: 1,
                },
            ],
            output_s3_uri: "s3://uploads/reviews/job-123/processed/stitched.wav".to_owned(),
        };

        assert_eq!(validate_input(input).unwrap().files[0].sequence, 2);
    }

    #[tokio::test]
    async fn returns_the_single_stitched_wav_location() {
        let output = handle(AudioProcessingInput {
            job_id: "job-123".to_owned(),
            files: vec![AudioSource {
                s3_uri: "s3://uploads/reviews/job-123/source.wav".to_owned(),
                sequence: 1,
            }],
            output_s3_uri: "s3://uploads/reviews/job-123/processed/stitched.wav".to_owned(),
        })
        .await
        .unwrap();

        assert_eq!(
            output.stitched_s3_uri,
            "s3://uploads/reviews/job-123/processed/stitched.wav"
        );
    }

    #[test]
    fn rejects_an_empty_file_list() {
        let error = validate_input(AudioProcessingInput {
            job_id: "job-123".to_owned(),
            files: vec![],
            output_s3_uri: "s3://uploads/reviews/job-123/processed/stitched.wav".to_owned(),
        })
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "audio processing requires at least one file"
        );
    }

    #[test]
    fn rejects_duplicate_sequence_values() {
        let error = validate_input(AudioProcessingInput {
            job_id: "job-123".to_owned(),
            files: vec![
                AudioSource {
                    s3_uri: "s3://uploads/reviews/job-123/source-1.wav".to_owned(),
                    sequence: 1,
                },
                AudioSource {
                    s3_uri: "s3://uploads/reviews/job-123/source-2.wav".to_owned(),
                    sequence: 1,
                },
            ],
            output_s3_uri: "s3://uploads/reviews/job-123/processed/stitched.wav".to_owned(),
        })
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "audio processing requires unique sequence values; 1 was repeated"
        );
    }
}
