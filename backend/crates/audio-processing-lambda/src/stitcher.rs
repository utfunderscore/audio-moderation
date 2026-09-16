use std::path::{Path, PathBuf};
use std::process::Stdio;

use tempfile::{NamedTempFile, TempDir};
use tokio::process::Command;

use crate::LocalAudioSegment;

const SAMPLE_RATE: u32 = 16_000;
const CHANNELS: u8 = 1;

#[derive(Debug, thiserror::Error)]
pub enum AudioStitchError {
    #[error("at least one audio segment is required")]
    NoSegments,
    #[error("audio stitching I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to start ffmpeg at {executable}: {source}")]
    StartFfmpeg {
        executable: PathBuf,
        source: std::io::Error,
    },
    #[error("ffmpeg failed while stitching audio: {0}")]
    StitchFailed(String),
    #[error("failed to persist the stitched WAV file: {0}")]
    Persist(#[from] tempfile::PersistError),
}

#[derive(Clone)]
pub struct AudioStitcher {
    ffmpeg_path: PathBuf,
}

pub struct StitchedWav {
    directory: TempDir,
    path: PathBuf,
}

impl StitchedWav {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn directory(&self) -> &Path {
        self.directory.path()
    }
}

impl Default for AudioStitcher {
    fn default() -> Self {
        Self::new("ffmpeg")
    }
}

impl AudioStitcher {
    pub fn new(ffmpeg_path: impl Into<PathBuf>) -> Self {
        Self {
            ffmpeg_path: ffmpeg_path.into(),
        }
    }

    pub async fn stitch(
        &self,
        mut segments: Vec<LocalAudioSegment>,
    ) -> Result<StitchedWav, AudioStitchError> {
        if segments.is_empty() {
            return Err(AudioStitchError::NoSegments);
        }

        segments.sort_by_key(|segment| (segment.sequence, segment.segment_id));

        let output_directory = tempfile::tempdir()?;
        let output_path = output_directory.path().join("stitched.wav");
        self.write_wav(&segments, &output_path).await?;

        Ok(StitchedWav {
            directory: output_directory,
            path: output_path,
        })
    }

    async fn write_wav(
        &self,
        segments: &[LocalAudioSegment],
        output_path: &Path,
    ) -> Result<(), AudioStitchError> {
        let output_directory = output_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let output_file = NamedTempFile::new_in(output_directory)?;

        let mut command = Command::new(&self.ffmpeg_path);
        command.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-xerror"]);
        for segment in segments {
            command.arg("-i").arg(segment.file.path());
        }

        let filter = concat_filter(segments.len());
        let child = command
            .args([
                "-filter_complex",
                &filter,
                "-map",
                "[out]",
                "-vn",
                "-c:a",
                "pcm_s16le",
                "-ar",
                &SAMPLE_RATE.to_string(),
                "-ac",
                &CHANNELS.to_string(),
                "-f",
                "wav",
                "-y",
            ])
            .arg(output_file.path())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|source| AudioStitchError::StartFfmpeg {
                executable: self.ffmpeg_path.clone(),
                source,
            })?;
        let output = child.wait_with_output().await?;

        if !output.status.success() {
            return Err(AudioStitchError::StitchFailed(ffmpeg_message(
                &output.stderr,
            )));
        }

        output_file.persist(output_path)?;
        Ok(())
    }
}

fn concat_filter(segment_count: usize) -> String {
    let mut filter = String::new();
    for index in 0..segment_count {
        filter.push_str(&format!(
            "[{index}:a:0]aformat=sample_fmts=s16:sample_rates={SAMPLE_RATE}:channel_layouts=mono,asetpts=PTS-STARTPTS[a{index}];"
        ));
    }
    for index in 0..segment_count {
        filter.push_str(&format!("[a{index}]"));
    }
    filter.push_str(&format!("concat=n={segment_count}:v=0:a=1[out]"));
    filter
}

fn ffmpeg_message(stderr: &[u8]) -> String {
    let message = String::from_utf8_lossy(stderr).trim().to_owned();
    if message.is_empty() {
        "ffmpeg exited without an error message".to_owned()
    } else {
        message
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::NamedTempFile;

    use super::{AudioStitchError, AudioStitcher, StitchedWav, concat_filter};
    use crate::LocalAudioSegment;

    #[tokio::test]
    async fn rejects_an_empty_segment_list() {
        let stitcher = AudioStitcher::new("missing-ffmpeg");
        let result = stitcher.stitch(Vec::new()).await;

        assert!(matches!(result, Err(AudioStitchError::NoSegments)));
    }

    #[tokio::test]
    async fn orders_equal_sequences_by_segment_id() {
        let directory = tempfile::tempdir().unwrap();
        let first = NamedTempFile::new_in(directory.path()).unwrap();
        let second = NamedTempFile::new_in(directory.path()).unwrap();
        write_test_wav(first.path(), 1_000, 160);
        write_test_wav(second.path(), -1_000, 160);

        let output = AudioStitcher::default()
            .stitch(vec![
                LocalAudioSegment {
                    segment_id: 2,
                    file: second,
                    sequence: 10,
                },
                LocalAudioSegment {
                    segment_id: 1,
                    file: first,
                    sequence: 10,
                },
            ])
            .await
            .unwrap();

        let samples = wav_samples(&read_output(&output));
        assert_eq!(&samples[..160], &[1_000; 160]);
        assert_eq!(&samples[160..], &[-1_000; 160]);
    }

    #[tokio::test]
    async fn stitches_segments_in_sequence_order() {
        let directory = tempfile::tempdir().unwrap();
        let early = NamedTempFile::new_in(directory.path()).unwrap();
        let late = NamedTempFile::new_in(directory.path()).unwrap();
        write_test_wav(early.path(), 1_000, 160);
        write_test_wav(late.path(), -1_000, 160);

        let output = AudioStitcher::default()
            .stitch(vec![
                LocalAudioSegment {
                    segment_id: 2,
                    file: late,
                    sequence: 20,
                },
                LocalAudioSegment {
                    segment_id: 1,
                    file: early,
                    sequence: 10,
                },
            ])
            .await
            .unwrap();

        let wav = read_output(&output);
        let samples = wav_samples(&wav);
        assert_eq!(samples.len(), 320);
        assert_eq!(samples[0], 1_000);
        assert_eq!(samples[159], 1_000);
        assert_eq!(samples[160], -1_000);
        assert_eq!(samples[319], -1_000);
    }

    #[tokio::test]
    async fn ignores_nonzero_input_pts() {
        let directory = tempfile::tempdir().unwrap();
        let source = NamedTempFile::new_in(directory.path()).unwrap();
        let shifted = NamedTempFile::new_in(directory.path()).unwrap();
        let following = NamedTempFile::new_in(directory.path()).unwrap();
        write_test_wav(source.path(), 1_000, 160);
        write_test_wav(following.path(), -1_000, 160);
        write_shifted_pts_audio(source.path(), shifted.path());

        let output = AudioStitcher::default()
            .stitch(vec![
                LocalAudioSegment {
                    segment_id: 1,
                    file: shifted,
                    sequence: 1,
                },
                LocalAudioSegment {
                    segment_id: 2,
                    file: following,
                    sequence: 2,
                },
            ])
            .await
            .unwrap();

        let samples = wav_samples(&read_output(&output));
        assert_eq!(samples.len(), 320);
        assert!(samples[..160].iter().all(|sample| *sample > 0));
        assert!(samples[160..].iter().all(|sample| *sample < 0));
    }

    #[tokio::test]
    async fn stitches_a_single_segment() {
        let directory = tempfile::tempdir().unwrap();
        let input = NamedTempFile::new_in(directory.path()).unwrap();
        write_test_wav(input.path(), 500, 160);

        let output = AudioStitcher::default()
            .stitch(vec![LocalAudioSegment {
                segment_id: 1,
                file: input,
                sequence: 10,
            }])
            .await
            .unwrap();

        assert_eq!(output.path().file_name().unwrap(), "stitched.wav");
        assert_ne!(output.directory(), directory.path());
        assert_eq!(wav_samples(&read_output(&output)), vec![500; 160]);
    }

    #[tokio::test]
    async fn normalizes_segments_to_sixteen_khz_mono_pcm() {
        let directory = tempfile::tempdir().unwrap();
        let stereo = NamedTempFile::new_in(directory.path()).unwrap();
        let low_rate = NamedTempFile::new_in(directory.path()).unwrap();
        write_test_wav_with_format(stereo.path(), 1_000, 480, 48_000, 2);
        write_test_wav_with_format(low_rate.path(), -1_000, 80, 8_000, 1);

        let output = AudioStitcher::default()
            .stitch(vec![
                LocalAudioSegment {
                    segment_id: 1,
                    file: stereo,
                    sequence: 10,
                },
                LocalAudioSegment {
                    segment_id: 2,
                    file: low_rate,
                    sequence: 20,
                },
            ])
            .await
            .unwrap();

        let wav = read_output(&output);
        assert_eq!(wav_format(&wav), (1, 1, 16_000, 16));
        let samples = wav_samples(&wav);
        assert_eq!(samples.len(), 320);
        assert!(samples[..160].iter().all(|sample| *sample > 0));
        assert!(samples[160..].iter().all(|sample| *sample < 0));
    }

    #[tokio::test]
    async fn reports_when_ffmpeg_cannot_be_started() {
        let directory = tempfile::tempdir().unwrap();
        let input = NamedTempFile::new_in(directory.path()).unwrap();
        let missing_ffmpeg = directory.path().join("missing-ffmpeg");
        write_test_wav(input.path(), 500, 160);

        let result = AudioStitcher::new(&missing_ffmpeg)
            .stitch(vec![LocalAudioSegment {
                segment_id: 1,
                file: input,
                sequence: 10,
            }])
            .await;

        assert!(matches!(
            result,
            Err(AudioStitchError::StartFfmpeg { executable, .. }) if executable == missing_ffmpeg
        ));
    }

    #[tokio::test]
    async fn reports_decode_failures() {
        let directory = tempfile::tempdir().unwrap();
        let invalid_input = NamedTempFile::new_in(directory.path()).unwrap();
        fs::write(invalid_input.path(), b"not audio").unwrap();

        let result = AudioStitcher::default()
            .stitch(vec![LocalAudioSegment {
                segment_id: 1,
                file: invalid_input,
                sequence: 10,
            }])
            .await;

        assert!(matches!(result, Err(AudioStitchError::StitchFailed(_))));
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    fn resets_each_input_pts_before_concatenating() {
        assert_eq!(
            concat_filter(2),
            "[0:a:0]aformat=sample_fmts=s16:sample_rates=16000:channel_layouts=mono,asetpts=PTS-STARTPTS[a0];[1:a:0]aformat=sample_fmts=s16:sample_rates=16000:channel_layouts=mono,asetpts=PTS-STARTPTS[a1];[a0][a1]concat=n=2:v=0:a=1[out]"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn uses_a_fallback_message_when_ffmpeg_writes_no_error() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let input = NamedTempFile::new_in(directory.path()).unwrap();
        let failing_ffmpeg = directory.path().join("failing-ffmpeg");
        write_test_wav(input.path(), 500, 160);
        fs::write(&failing_ffmpeg, "#!/bin/sh\nexit 1\n").unwrap();
        fs::set_permissions(&failing_ffmpeg, fs::Permissions::from_mode(0o755)).unwrap();

        let result = AudioStitcher::new(failing_ffmpeg)
            .stitch(vec![LocalAudioSegment {
                segment_id: 1,
                file: input,
                sequence: 10,
            }])
            .await;

        assert!(matches!(
            result,
            Err(AudioStitchError::StitchFailed(message))
                if message == "ffmpeg exited without an error message"
        ));
    }

    fn write_test_wav(path: &Path, sample: i16, sample_count: u32) {
        write_test_wav_with_format(path, sample, sample_count, 16_000, 1);
    }

    fn write_shifted_pts_audio(source: &Path, destination: &Path) {
        let status = std::process::Command::new("ffmpeg")
            .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-i"])
            .arg(source)
            .args([
                "-output_ts_offset",
                "5",
                "-c:a",
                "pcm_s16le",
                "-f",
                "matroska",
                "-y",
            ])
            .arg(destination)
            .status()
            .unwrap();

        assert!(status.success());
    }

    fn read_output(output: &StitchedWav) -> Vec<u8> {
        fs::read(output.path()).unwrap()
    }

    fn write_test_wav_with_format(
        path: &Path,
        sample: i16,
        frame_count: u32,
        sample_rate: u32,
        channels: u16,
    ) {
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
        fs::write(path, wav).unwrap();
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
