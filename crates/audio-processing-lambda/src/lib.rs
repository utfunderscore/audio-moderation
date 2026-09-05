use std::error::Error as StdError;
use std::future::Future;
use std::path::Path;

use aws_sdk_s3::Client as S3Client;
use aws_sdk_s3::primitives::ByteStream;
use lambda_runtime::Error;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use tracing::{error, info};

use crate::stitcher::AudioStitcher;

pub mod stitcher;

const STITCHED_WAV_CONTENT_TYPE: &str = "audio/wav";

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

pub type S3StorageError = Box<dyn StdError + Send + Sync>;

pub trait S3Storage: Send + Sync {
    fn download(
        &self,
        location: &S3Location,
        destination: &Path,
    ) -> impl Future<Output = Result<(), S3StorageError>> + Send;

    fn upload(
        &self,
        location: &S3Location,
        source: &Path,
        content_type: &str,
    ) -> impl Future<Output = Result<(), S3StorageError>> + Send;
}

#[derive(Clone)]
pub struct AwsS3Storage {
    client: S3Client,
}

impl AwsS3Storage {
    pub fn new(client: S3Client) -> Self {
        Self { client }
    }
}

impl S3Storage for AwsS3Storage {
    async fn download(
        &self,
        location: &S3Location,
        destination: &Path,
    ) -> Result<(), S3StorageError> {
        let object = self
            .client
            .get_object()
            .bucket(location.bucket())
            .key(location.key())
            .send()
            .await?;
        let bytes = object.body.collect().await?.into_bytes();
        tokio::fs::write(destination, bytes).await?;
        Ok(())
    }

    async fn upload(
        &self,
        location: &S3Location,
        source: &Path,
        content_type: &str,
    ) -> Result<(), S3StorageError> {
        let body = ByteStream::from_path(source).await?;
        self.client
            .put_object()
            .bucket(location.bucket())
            .key(location.key())
            .content_type(content_type)
            .body(body)
            .send()
            .await?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct AudioProcessingHandler<S> {
    s3: S,
    stitcher: AudioStitcher,
}

impl<S> AudioProcessingHandler<S> {
    pub fn new(s3: S) -> Self {
        Self {
            s3,
            stitcher: AudioStitcher::default(),
        }
    }

    #[cfg(test)]
    fn with_stitcher(s3: S, stitcher: AudioStitcher) -> Self {
        Self { s3, stitcher }
    }
}

impl<S: S3Storage> AudioProcessingHandler<S> {
    pub async fn handle(
        &self,
        input: AudioProcessingInput,
    ) -> Result<AudioProcessingOutput, Error> {
        let input = validate_input(input)?;
        info!(
            jobId = input.job_id,
            fileCount = input.files.len(),
            outputBucket = input.output.bucket(),
            outputKey = input.output.key(),
            "received audio stitching request"
        );

        let mut segments = Vec::with_capacity(input.files.len());
        for (index, source) in input.files.iter().enumerate() {
            let file = NamedTempFile::new()?;
            info!(
                jobId = input.job_id,
                sourceBucket = source.location.bucket(),
                sourceKey = source.location.key(),
                sequence = source.sequence,
                segmentId = index + 1,
                "downloading source audio"
            );
            if let Err(source_error) = self.s3.download(&source.location, file.path()).await {
                error!(
                    jobId = input.job_id,
                    sourceBucket = source.location.bucket(),
                    sourceKey = source.location.key(),
                    sequence = source.sequence,
                    error = ?source_error,
                    "failed to download source audio"
                );
                return Err(AudioProcessingError::Download {
                    location: source.location.clone(),
                    source: source_error,
                }
                .into());
            }
            segments.push(LocalAudioSegment {
                segment_id: u64::try_from(index + 1).expect("segment index exceeds u64"),
                file,
                sequence: source.sequence,
            });
        }

        let stitched = self.stitcher.stitch(segments).await.map_err(|stitch_error| {
            error!(jobId = input.job_id, error = ?stitch_error, "failed to stitch source audio");
            AudioProcessingError::Stitch(stitch_error)
        })?;

        info!(
            jobId = input.job_id,
            outputBucket = input.output.bucket(),
            outputKey = input.output.key(),
            contentType = STITCHED_WAV_CONTENT_TYPE,
            "uploading stitched audio"
        );
        if let Err(source_error) = self
            .s3
            .upload(&input.output, stitched.path(), STITCHED_WAV_CONTENT_TYPE)
            .await
        {
            error!(
                jobId = input.job_id,
                outputBucket = input.output.bucket(),
                outputKey = input.output.key(),
                error = ?source_error,
                "failed to upload stitched audio"
            );
            return Err(AudioProcessingError::Upload {
                location: input.output,
                source: source_error,
            }
            .into());
        }

        info!(
            jobId = input.job_id,
            outcome = "stitched",
            "stitched and uploaded audio"
        );
        Ok(AudioProcessingOutput {
            job_id: input.job_id,
            stitched_s3_uri: input.output_s3_uri,
        })
    }
}

struct ValidatedAudioProcessingInput {
    job_id: String,
    files: Vec<ValidatedAudioSource>,
    output: S3Location,
    output_s3_uri: String,
}

struct ValidatedAudioSource {
    location: S3Location,
    sequence: u32,
}

fn validate_input(
    input: AudioProcessingInput,
) -> Result<ValidatedAudioProcessingInput, AudioProcessingError> {
    if input.files.is_empty() {
        return Err(AudioProcessingError::NoFiles);
    }

    let output = S3Location::parse(&input.output_s3_uri)?;
    let mut files = Vec::with_capacity(input.files.len());
    for file in input.files {
        let location = S3Location::parse(&file.s3_uri)?;
        if files
            .iter()
            .any(|previous: &ValidatedAudioSource| previous.sequence == file.sequence)
        {
            return Err(AudioProcessingError::DuplicateSequence(file.sequence));
        }
        files.push(ValidatedAudioSource {
            location,
            sequence: file.sequence,
        });
    }

    Ok(ValidatedAudioProcessingInput {
        job_id: input.job_id,
        files,
        output,
        output_s3_uri: input.output_s3_uri,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3Location {
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
        if !is_valid_bucket(bucket) || key.is_empty() || key.chars().any(char::is_control) {
            return Err(AudioProcessingError::InvalidS3Uri(uri.to_owned()));
        }

        Ok(Self {
            bucket: bucket.to_owned(),
            key: key.to_owned(),
        })
    }

    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    pub fn key(&self) -> &str {
        &self.key
    }
}

fn is_valid_bucket(bucket: &str) -> bool {
    (3..=63).contains(&bucket.len())
        && !bucket.parse::<std::net::Ipv4Addr>().is_ok()
        && bucket.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'.' || byte == b'-'
        })
        && bucket
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bucket
            .bytes()
            .next_back()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && !bucket.contains("..")
}

#[derive(Debug, thiserror::Error)]
enum AudioProcessingError {
    #[error("audio processing requires at least one file")]
    NoFiles,
    #[error("audio processing requires unique sequence values; {0} was repeated")]
    DuplicateSequence(u32),
    #[error("invalid S3 URI {0}")]
    InvalidS3Uri(String),
    #[error("failed to download S3 object {location}: {source}")]
    Download {
        location: S3Location,
        #[source]
        source: S3StorageError,
    },
    #[error("failed to stitch source audio: {0}")]
    Stitch(#[source] stitcher::AudioStitchError),
    #[error("failed to upload S3 object {location}: {source}")]
    Upload {
        location: S3Location,
        #[source]
        source: S3StorageError,
    },
}

impl std::fmt::Display for S3Location {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "s3://{}/{}", self.bucket, self.key)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::path::Path;
    use std::sync::Mutex;

    use super::*;

    #[test]
    fn parses_bucket_and_nested_object_key() {
        assert_eq!(
            S3Location::parse("s3://uploads.example/reviews/job-123/source.wav").unwrap(),
            S3Location {
                bucket: "uploads.example".to_owned(),
                key: "reviews/job-123/source.wav".to_owned(),
            }
        );
    }

    #[test]
    fn rejects_malformed_s3_uris() {
        for uri in [
            "https://uploads/reviews/source.wav",
            "s3://uploads",
            "s3:///reviews/source.wav",
            "s3://UPPERCASE/reviews/source.wav",
            "s3://uploads/",
            "s3://uploads/reviews/source\n.wav",
        ] {
            assert!(matches!(
                S3Location::parse(uri),
                Err(AudioProcessingError::InvalidS3Uri(_))
            ));
        }
    }

    #[test]
    fn retains_supplied_file_order_and_sequences_when_validating() {
        let input = validate_input(input(vec![
            ("s3://uploads/reviews/source-2.wav", 2),
            ("s3://uploads/reviews/source-1.wav", 1),
        ]))
        .unwrap();

        assert_eq!(input.files[0].sequence, 2);
        assert_eq!(input.files[1].sequence, 1);
    }

    #[test]
    fn rejects_an_empty_file_list() {
        assert!(matches!(
            validate_input(input(Vec::new())),
            Err(AudioProcessingError::NoFiles)
        ));
    }

    #[test]
    fn rejects_duplicate_sequence_values() {
        assert!(matches!(
            validate_input(input(vec![
                ("s3://uploads/reviews/source-1.wav", 1),
                ("s3://uploads/reviews/source-2.wav", 1),
            ])),
            Err(AudioProcessingError::DuplicateSequence(1))
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn downloads_stitches_and_uploads_before_returning_output() {
        let s3 = MockS3::with_objects(HashMap::from([
            (
                "s3://uploads/reviews/source-2.wav".to_owned(),
                b"second".to_vec(),
            ),
            (
                "s3://uploads/reviews/source-1.wav".to_owned(),
                b"first".to_vec(),
            ),
        ]));
        let directory = tempfile::tempdir().unwrap();
        let fake_ffmpeg = directory.path().join("ffmpeg");
        write_fake_ffmpeg(&fake_ffmpeg);
        let handler = AudioProcessingHandler::with_stitcher(s3, AudioStitcher::new(fake_ffmpeg));

        let output = handler
            .handle(input(vec![
                ("s3://uploads/reviews/source-2.wav", 2),
                ("s3://uploads/reviews/source-1.wav", 1),
            ]))
            .await
            .unwrap();

        assert_eq!(output.job_id, "job-123");
        assert_eq!(output.stitched_s3_uri, "s3://uploads/reviews/stitched.wav");
        assert_eq!(
            handler.s3.uploads(),
            vec![UploadedObject {
                location: "s3://uploads/reviews/stitched.wav".to_owned(),
                content_type: "audio/wav".to_owned(),
                body: b"first".to_vec(),
            }]
        );
    }

    #[tokio::test]
    async fn returns_a_download_failure_without_uploading() {
        let s3 = MockS3::default();
        let handler = AudioProcessingHandler::new(s3);

        let error = handler
            .handle(input(vec![("s3://uploads/reviews/missing.wav", 1)]))
            .await
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("failed to download S3 object s3://uploads/reviews/missing.wav")
        );
        assert!(handler.s3.uploads().is_empty());
    }

    fn input(files: Vec<(&str, u32)>) -> AudioProcessingInput {
        AudioProcessingInput {
            job_id: "job-123".to_owned(),
            files: files
                .into_iter()
                .map(|(s3_uri, sequence)| AudioSource {
                    s3_uri: s3_uri.to_owned(),
                    sequence,
                })
                .collect(),
            output_s3_uri: "s3://uploads/reviews/stitched.wav".to_owned(),
        }
    }

    #[derive(Default)]
    struct MockS3 {
        objects: HashMap<String, Vec<u8>>,
        uploads: Mutex<Vec<UploadedObject>>,
    }

    impl MockS3 {
        fn with_objects(objects: HashMap<String, Vec<u8>>) -> Self {
            Self {
                objects,
                uploads: Mutex::default(),
            }
        }

        fn uploads(&self) -> Vec<UploadedObject> {
            self.uploads.lock().unwrap().clone()
        }
    }

    impl S3Storage for MockS3 {
        async fn download(
            &self,
            location: &S3Location,
            destination: &Path,
        ) -> Result<(), S3StorageError> {
            let object = self.objects.get(&location.to_string()).ok_or_else(|| {
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "object not found",
                )) as S3StorageError
            })?;
            fs::write(destination, object)?;
            Ok(())
        }

        async fn upload(
            &self,
            location: &S3Location,
            source: &Path,
            content_type: &str,
        ) -> Result<(), S3StorageError> {
            self.uploads.lock().unwrap().push(UploadedObject {
                location: location.to_string(),
                content_type: content_type.to_owned(),
                body: fs::read(source)?,
            });
            Ok(())
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct UploadedObject {
        location: String,
        content_type: String,
        body: Vec<u8>,
    }

    #[cfg(unix)]
    fn write_fake_ffmpeg(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        fs::write(
            path,
            "#!/bin/sh\ninput=''\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    -i) if [ -z \"$input\" ]; then input=\"$2\"; fi; shift 2 ;;\n    *) output=\"$1\"; shift ;;\n  esac\ndone\ncp \"$input\" \"$output\"\n",
        )
        .unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}
