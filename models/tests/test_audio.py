import io
import subprocess
import wave
from pathlib import Path
from unittest.mock import Mock

import pytest

import socialguard_models.audio as audio


def write_wav(path: Path, samples: bytes, *, channels: int = 1, width: int = 2, rate: int = 16_000) -> None:
    with wave.open(str(path), "wb") as output:
        output.setnchannels(channels)
        output.setsampwidth(width)
        output.setframerate(rate)
        output.writeframes(samples)


def test_download_stops_at_byte_limit(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    monkeypatch.setattr(audio, "urlopen", lambda *args, **kwargs: io.BytesIO(b"12345"))
    with pytest.raises(ValueError, match="byte limit"):
        audio.download_audio("https://example.com/audio", tmp_path / "source", max_bytes=4)
    audio.download_audio("https://example.com/audio", tmp_path / "source", max_bytes=5)
    assert (tmp_path / "source").read_bytes() == b"12345"


def test_normalization_explicitly_bounds_and_encodes_pcm16(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    run = Mock()
    monkeypatch.setattr(audio.subprocess, "run", run)
    audio.normalize_audio(tmp_path / "source", tmp_path / "out.wav", max_seconds=300)
    command = run.call_args.args[0]
    for flag, value in [("-ac", "1"), ("-ar", "16000"), ("-t", "301"), ("-c:a", "pcm_s16le")]:
        assert command[command.index(flag) + 1] == value
    assert run.call_args.kwargs == {"check": True, "timeout": 300}


@pytest.mark.parametrize("kwargs", [{"channels": 2}, {"width": 1}, {"rate": 8_000}])
def test_validation_rejects_wrong_wav_format(tmp_path: Path, kwargs: dict[str, int]) -> None:
    path = tmp_path / "audio.wav"
    write_wav(path, b"\0" * 4, **kwargs)
    with pytest.raises(RuntimeError, match="normalize"):
        audio.validate_wav(path, max_seconds=1)


def test_validation_rejects_empty_and_overlong_audio(tmp_path: Path) -> None:
    path = tmp_path / "audio.wav"
    for samples in (0, 16_001):
        write_wav(path, bytes(samples * 2))
        with pytest.raises(ValueError):
            audio.validate_wav(path, max_seconds=1)
    write_wav(path, bytes(32_000))
    assert audio.validate_wav(path, max_seconds=1) == 16_000


def test_real_ffmpeg_normalizes_stereo_and_rejects_overlength(tmp_path: Path) -> None:
    source, destination = tmp_path / "source.wav", tmp_path / "normalized.wav"
    write_wav(source, bytes(8_000 * 2 * 2 * 2), channels=2, rate=8_000)
    audio.normalize_audio(source, destination, max_seconds=1)
    with pytest.raises(ValueError, match="1 seconds"):
        audio.validate_wav(destination, max_seconds=1)
    source.write_bytes(b"invalid audio")
    destination.unlink()
    with pytest.raises(subprocess.CalledProcessError):
        audio.normalize_audio(source, destination, max_seconds=1)
