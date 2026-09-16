"""Bounded audio ingestion shared by GPU model runtimes."""

import subprocess
import wave
from pathlib import Path
from urllib.parse import urlsplit
from urllib.request import Request, urlopen

SAMPLE_RATE = 16_000


def download_audio(audio_url: str, destination: Path, *, max_bytes: int) -> None:
    parsed = urlsplit(audio_url)
    if parsed.scheme != "https" or not parsed.netloc:
        raise ValueError("audio URL must be an absolute HTTPS URL")

    request = Request(audio_url, headers={"User-Agent": "socialguard-models/0.1"})
    downloaded_bytes = 0
    with urlopen(request, timeout=120) as response, destination.open("wb") as output:
        while chunk := response.read(1024 * 1024):
            downloaded_bytes += len(chunk)
            if downloaded_bytes > max_bytes:
                raise ValueError("audio file exceeds the configured byte limit")
            output.write(chunk)


def normalize_audio(source: Path, destination: Path, *, max_seconds: int) -> None:
    """Downmix and decode only enough audio to detect an overlength recording."""
    subprocess.run(
        [
            "ffmpeg", "-nostdin", "-loglevel", "error", "-i", str(source),
            "-ac", "1", "-ar", str(SAMPLE_RATE), "-t", str(max_seconds + 1),
            "-c:a", "pcm_s16le", "-f", "wav", str(destination),
        ],
        check=True,
        timeout=300,
    )


def validate_wav(audio_path: Path, *, max_seconds: int) -> int:
    """Validate normalized PCM16 audio and return its sample count."""
    with wave.open(str(audio_path), "rb") as wav_file:
        if (
            wav_file.getframerate() != SAMPLE_RATE
            or wav_file.getnchannels() != 1
            or wav_file.getsampwidth() != 2
            or wav_file.getcomptype() != "NONE"
        ):
            raise RuntimeError("failed to normalize audio to mono 16 kHz PCM16")
        frame_count = wav_file.getnframes()
    if frame_count == 0:
        raise ValueError("audio must contain at least one sample")
    if frame_count > max_seconds * SAMPLE_RATE:
        raise ValueError(f"audio must not exceed {max_seconds} seconds")
    return frame_count
