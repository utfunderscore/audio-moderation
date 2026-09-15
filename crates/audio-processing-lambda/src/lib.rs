use std::error::Error as StdError;
use std::future::Future;
use std::path::Path;

use aws_sdk_s3::Client as S3Client;
use aws_sdk_s3::primitives::ByteStream;
use lambda_runtime::Error;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use tracing::{error, info};

use database::{PipelineTaskError, PipelineTaskOutcome, PipelineTaskStore};

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

/// The worker also accepts lifecycle events from the state machine and its
/// terminal-execution EventBridge reconciler. This keeps workflow state in the
/// same database transaction boundary as the audio worker.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum AudioWorkerInput {
    Audio(AudioProcessingInput),
    Lifecycle(WorkflowLifecycleInput),
    ExecutionStatus(ExecutionStatusEvent),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowLifecycleInput {
    pub job_id: String,
    pub outcome: WorkflowOutcome,
    pub error: Option<String>,
    pub cause: Option<String>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkflowOutcome {
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
}

impl From<WorkflowOutcome> for PipelineTaskOutcome {
    fn from(outcome: WorkflowOutcome) -> Self {
        match outcome {
            WorkflowOutcome::Succeeded => Self::Succeeded,
            WorkflowOutcome::Failed => Self::Failed,
            WorkflowOutcome::TimedOut => Self::TimedOut,
            WorkflowOutcome::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ExecutionStatusEvent {
    pub detail: ExecutionStatusDetail,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionStatusDetail {
    pub status: String,
    pub input: String,
    pub error: Option<String>,
    pub cause: Option<String>,
}

impl ExecutionStatusEvent {
    pub fn lifecycle(self) -> Result<WorkflowLifecycleInput, serde_json::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Input {
            job_id: String,
        }
        let input: Input = serde_json::from_str(&self.detail.input)?;
        let outcome = match self.detail.status.as_str() {
            "SUCCEEDED" => WorkflowOutcome::Succeeded,
            "FAILED" => WorkflowOutcome::Failed,
            "TIMED_OUT" => WorkflowOutcome::TimedOut,
            "ABORTED" => WorkflowOutcome::Cancelled,
            _ => WorkflowOutcome::Failed,
        };
        Ok(WorkflowLifecycleInput {
            job_id: input.job_id,
            outcome,
            error: self.detail.error,
            cause: self.detail.cause,
        })
    }
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

/// Persistence needed by the audio-processing worker.
pub trait AudioTaskStore: Send + Sync {
    fn start_audio_processing(
        &self,
        task_id: i32,
    ) -> impl Future<Output = Result<(), PipelineTaskError>> + Send;

    fn complete_audio_processing(
        &self,
        task_id: i32,
        stitched_audio_s3_uri: String,
    ) -> impl Future<Output = Result<(), PipelineTaskError>> + Send;

    fn fail_audio_processing(
        &self,
        task_id: i32,
        error_code: String,
        error_message: String,
    ) -> impl Future<Output = Result<(), PipelineTaskError>> + Send;
}

impl AudioTaskStore for PipelineTaskStore {
    async fn start_audio_processing(&self, task_id: i32) -> Result<(), PipelineTaskError> {
        PipelineTaskStore::start_audio_processing(self, task_id).await
    }

    async fn complete_audio_processing(
        &self,
        task_id: i32,
        stitched_audio_s3_uri: String,
    ) -> Result<(), PipelineTaskError> {
        PipelineTaskStore::complete_audio_processing(self, task_id, &stitched_audio_s3_uri).await
    }

    async fn fail_audio_processing(
        &self,
        task_id: i32,
        error_code: String,
        error_message: String,
    ) -> Result<(), PipelineTaskError> {
        PipelineTaskStore::fail_audio_processing(self, task_id, &error_code, &error_message).await
    }
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
pub struct AudioProcessingHandler<S, D> {
    s3: S,
    database: D,
    stitcher: AudioStitcher,
}

impl<S, D> AudioProcessingHandler<S, D> {
    pub fn new(s3: S, database: D) -> Self {
        Self {
            s3,
            database,
            stitcher: AudioStitcher::default(),
        }
    }
}

impl<S: S3Storage, D: AudioTaskStore> AudioProcessingHandler<S, D> {
    pub async fn handle(
        &self,
        input: AudioProcessingInput,
    ) -> Result<AudioProcessingOutput, Error> {
        let input = validate_input(input)?;
        let task_id = input
            .job_id
            .parse()
            .map_err(|_| AudioProcessingError::InvalidJobId(input.job_id.clone()))?;
        self.database
            .start_audio_processing(task_id)
            .await
            .map_err(AudioProcessingError::StartAudioProcessing)?;
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
                self.record_failure(task_id, "AUDIO_DOWNLOAD_FAILED", &source_error.to_string())
                    .await;
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

        let stitched = match self.stitcher.stitch(segments).await {
            Ok(stitched) => stitched,
            Err(stitch_error) => {
                error!(jobId = input.job_id, error = ?stitch_error, "failed to stitch source audio");
                self.record_failure(task_id, "AUDIO_STITCH_FAILED", &stitch_error.to_string())
                    .await;
                return Err(AudioProcessingError::Stitch(stitch_error).into());
            }
        };

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
            self.record_failure(task_id, "AUDIO_UPLOAD_FAILED", &source_error.to_string())
                .await;
            return Err(AudioProcessingError::Upload {
                location: input.output,
                source: source_error,
            }
            .into());
        }

        self.database
            .complete_audio_processing(task_id, input.output_s3_uri.clone())
            .await
            .map_err(AudioProcessingError::CompleteAudioProcessing)?;

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

    async fn record_failure(&self, task_id: i32, error_code: &str, error_message: &str) {
        if let Err(error) = self
            .database
            .fail_audio_processing(task_id, error_code.to_owned(), error_message.to_owned())
            .await
        {
            error!(taskId = task_id, error = ?error, "failed to persist audio worker failure");
        }
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
        && bucket.parse::<std::net::Ipv4Addr>().is_err()
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
    #[error("invalid pipeline job ID {0}")]
    InvalidJobId(String),
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
    #[error("failed to start audio processing: {0}")]
    StartAudioProcessing(#[source] PipelineTaskError),
    #[error("failed to complete audio processing: {0}")]
    CompleteAudioProcessing(#[source] PipelineTaskError),
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
    use std::sync::{Arc, Mutex};

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

    #[tokio::test]
    async fn uploads_sequence_ordered_normalized_wav_and_returns_output() {
        let s3 = MockS3::with_objects(HashMap::from([
            (
                "s3://uploads/reviews/source-2.wav".to_owned(),
                test_wav(1_000, 480, 48_000, 2),
            ),
            (
                "s3://uploads/reviews/source-1.wav".to_owned(),
                test_wav(-1_000, 80, 8_000, 1),
            ),
        ]));
        let handler = AudioProcessingHandler::new(s3, MockDatabase::default());

        let output = handler
            .handle(input(vec![
                ("s3://uploads/reviews/source-2.wav", 2),
                ("s3://uploads/reviews/source-1.wav", 1),
            ]))
            .await
            .unwrap();

        assert_eq!(output.job_id, "42");
        assert_eq!(output.stitched_s3_uri, "s3://uploads/reviews/stitched.wav");
        let uploads = handler.s3.uploads();
        assert_eq!(uploads.len(), 1);
        assert_eq!(uploads[0].location, "s3://uploads/reviews/stitched.wav");
        assert_eq!(uploads[0].content_type, "audio/wav");
        assert_eq!(wav_format(&uploads[0].body), (1, 1, 16_000, 16));
        let samples = wav_samples(&uploads[0].body);
        assert_eq!(samples.len(), 320);
        assert!(samples[..160].iter().all(|sample| *sample < 0));
        assert!(samples[160..].iter().all(|sample| *sample > 0));
    }

    #[tokio::test]
    async fn returns_a_download_failure_without_uploading() {
        let s3 = MockS3::default();
        let handler = AudioProcessingHandler::new(s3, MockDatabase::default());

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

    #[tokio::test]
    async fn returns_a_stitch_failure_without_uploading() {
        let s3 = MockS3::with_objects(HashMap::from([(
            "s3://uploads/reviews/invalid.wav".to_owned(),
            b"not audio".to_vec(),
        )]));
        let handler = AudioProcessingHandler::new(s3, MockDatabase::default());

        let error = handler
            .handle(input(vec![("s3://uploads/reviews/invalid.wav", 1)]))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("failed to stitch source audio"));
        assert!(handler.s3.uploads().is_empty());
    }

    fn input(files: Vec<(&str, u32)>) -> AudioProcessingInput {
        AudioProcessingInput {
            job_id: "42".to_owned(),
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

    #[derive(Default)]
    struct MockDatabase {
        calls: Mutex<Vec<&'static str>>,
        fail_completion: bool,
        events: Option<Arc<Mutex<Vec<&'static str>>>>,
    }

    impl AudioTaskStore for MockDatabase {
        async fn start_audio_processing(&self, _task_id: i32) -> Result<(), PipelineTaskError> {
            self.calls.lock().unwrap().push("start");
            if let Some(events) = &self.events {
                events.lock().unwrap().push("start");
            }
            Ok(())
        }

        async fn complete_audio_processing(
            &self,
            _task_id: i32,
            _stitched_audio_s3_uri: String,
        ) -> Result<(), PipelineTaskError> {
            self.calls.lock().unwrap().push("complete");
            if let Some(events) = &self.events {
                events.lock().unwrap().push("complete");
            }
            if self.fail_completion {
                Err(PipelineTaskError::AudioOutputConflict)
            } else {
                Ok(())
            }
        }

        async fn fail_audio_processing(
            &self,
            _task_id: i32,
            _error_code: String,
            _error_message: String,
        ) -> Result<(), PipelineTaskError> {
            self.calls.lock().unwrap().push("fail");
            Ok(())
        }
    }

    #[tokio::test]
    async fn starts_before_work_and_completes_after_upload() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let s3 = RecordingS3 {
            inner: MockS3::with_objects(HashMap::from([(
                "s3://uploads/reviews/source.wav".to_owned(),
                test_wav(1_000, 80, 16_000, 1),
            )])),
            events: Arc::clone(&events),
        };
        let database = MockDatabase {
            events: Some(Arc::clone(&events)),
            ..Default::default()
        };
        let handler = AudioProcessingHandler::new(s3, database);

        handler
            .handle(input(vec![("s3://uploads/reviews/source.wav", 1)]))
            .await
            .unwrap();

        assert_eq!(
            *handler.database.calls.lock().unwrap(),
            ["start", "complete"]
        );
        assert_eq!(
            *events.lock().unwrap(),
            ["start", "download", "upload", "complete"]
        );
    }

    struct RecordingS3 {
        inner: MockS3,
        events: Arc<Mutex<Vec<&'static str>>>,
    }

    impl S3Storage for RecordingS3 {
        async fn download(
            &self,
            location: &S3Location,
            destination: &Path,
        ) -> Result<(), S3StorageError> {
            self.events.lock().unwrap().push("download");
            self.inner.download(location, destination).await
        }

        async fn upload(
            &self,
            location: &S3Location,
            source: &Path,
            content_type: &str,
        ) -> Result<(), S3StorageError> {
            self.events.lock().unwrap().push("upload");
            self.inner.upload(location, source, content_type).await
        }
    }

    #[tokio::test]
    async fn reports_completion_failure_after_upload() {
        let s3 = MockS3::with_objects(HashMap::from([(
            "s3://uploads/reviews/source.wav".to_owned(),
            test_wav(1_000, 80, 16_000, 1),
        )]));
        let handler = AudioProcessingHandler::new(
            s3,
            MockDatabase {
                fail_completion: true,
                ..Default::default()
            },
        );

        assert!(
            handler
                .handle(input(vec![("s3://uploads/reviews/source.wav", 1)]))
                .await
                .is_err()
        );
        assert_eq!(handler.s3.uploads().len(), 1);
        assert_eq!(
            *handler.database.calls.lock().unwrap(),
            ["start", "complete"]
        );
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct UploadedObject {
        location: String,
        content_type: String,
        body: Vec<u8>,
    }

    fn test_wav(sample: i16, frame_count: u32, sample_rate: u32, channels: u16) -> Vec<u8> {
        let block_align = channels * 2;
        let data_size = frame_count * u32::from(block_align);
        let mut wav = Vec::with_capacity(44 + data_size as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_size).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&channels.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&(sample_rate * u32::from(block_align)).to_le_bytes());
        wav.extend_from_slice(&block_align.to_le_bytes());
        wav.extend_from_slice(&16_u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());
        for _ in 0..frame_count * u32::from(channels) {
            wav.extend_from_slice(&sample.to_le_bytes());
        }
        wav
    }

    fn wav_format(wav: &[u8]) -> (u16, u16, u32, u16) {
        let format = wav_chunk(wav, b"fmt ");
        (
            u16::from_le_bytes(format[0..2].try_into().unwrap()),
            u16::from_le_bytes(format[2..4].try_into().unwrap()),
            u32::from_le_bytes(format[4..8].try_into().unwrap()),
            u16::from_le_bytes(format[14..16].try_into().unwrap()),
        )
    }

    fn wav_samples(wav: &[u8]) -> Vec<i16> {
        wav_chunk(wav, b"data")
            .chunks_exact(2)
            .map(|sample| i16::from_le_bytes([sample[0], sample[1]]))
            .collect()
    }

    fn wav_chunk<'a>(wav: &'a [u8], expected_id: &[u8; 4]) -> &'a [u8] {
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        let mut chunk_offset = 12;
        loop {
            let chunk_size =
                u32::from_le_bytes(wav[chunk_offset + 4..chunk_offset + 8].try_into().unwrap())
                    as usize;
            if &wav[chunk_offset..chunk_offset + 4] == expected_id {
                return &wav[chunk_offset + 8..chunk_offset + 8 + chunk_size];
            }
            chunk_offset += 8 + chunk_size + (chunk_size % 2);
        }
    }
}
