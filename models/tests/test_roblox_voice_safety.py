import asyncio
import io
import subprocess
import sys
import wave
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import AsyncMock, Mock

import pytest

import socialguard_models.moderation.models.roblox_voice_safety as roblox
from socialguard_models.moderation.models.registry import MODEL_SUBMITTERS
from socialguard_models.moderation.models.roblox_voice_safety_scores import EXPECTED_LABELS, map_scores


def wav_bytes(pcm: bytes) -> bytes:
    buffer = io.BytesIO()
    with wave.open(buffer, "wb") as output:
        output.setnchannels(1)
        output.setsampwidth(2)
        output.setframerate(16_000)
        output.writeframes(pcm)
    return buffer.getvalue()


def test_real_audio_path_includes_late_audio_and_cleans_up(monkeypatch: pytest.MonkeyPatch) -> None:
    worker = roblox.RobloxVoiceSafety._get_user_cls()()
    labels = sorted(EXPECTED_LABELS)
    observed = []
    paths = []

    def normalize(source: Path, destination: Path, *, max_seconds: int) -> None:
        paths.extend([source, destination])
        destination.write_bytes(source.read_bytes())

    def infer(pcm: bytes):
        observed.append(pcm)
        values = [0.01] * 8
        if b"\x01\x01" in pcm:
            values[labels.index("ABUSE_TYPE_HARASSMENT")] = 0.98
        return map_scores(labels, values)

    monkeypatch.setattr(roblox, "normalize_audio", normalize)
    monkeypatch.setattr(worker, "_infer_window", infer)
    pcm = bytes(35 * 16_000 * 2) + b"\x01\x01" * 16_000
    scores = worker.moderate_bytes(wav_bytes(pcm), "unused transcript")
    assert scores["harassment_or_abuse"] == 0.98
    assert len(observed) == 3
    assert all(len(window) == 240_000 * 2 for window in observed)
    assert all(not path.exists() for path in paths)


def test_inference_failure_cleans_temporary_files(monkeypatch: pytest.MonkeyPatch) -> None:
    worker = roblox.RobloxVoiceSafety._get_user_cls()()
    paths = []

    def normalize(source: Path, destination: Path, *, max_seconds: int) -> None:
        paths.extend([source, destination])
        destination.write_bytes(source.read_bytes())

    monkeypatch.setattr(roblox, "normalize_audio", normalize)
    monkeypatch.setattr(worker, "_infer_window", Mock(side_effect=RuntimeError("inference failed")))
    with pytest.raises(RuntimeError, match="inference failed"):
        worker.moderate_bytes(wav_bytes(bytes(32_000)))
    assert all(not path.exists() for path in paths)


def test_bytes_size_limit_precedes_normalization(monkeypatch: pytest.MonkeyPatch) -> None:
    worker = roblox.RobloxVoiceSafety._get_user_cls()()
    monkeypatch.setattr(roblox, "MAX_AUDIO_BYTES", 4)
    infer = Mock()
    monkeypatch.setattr(worker, "_moderate_file", infer)
    with pytest.raises(ValueError, match="512 MiB"):
        worker.moderate_bytes(b"12345")
    infer.assert_not_called()


def test_registry_submitter_returns_handle_without_waiting(monkeypatch: pytest.MonkeyPatch) -> None:
    handle = Mock()
    spawn = AsyncMock(return_value=handle)
    worker = SimpleNamespace(moderate=SimpleNamespace(spawn=SimpleNamespace(aio=spawn)))
    monkeypatch.setattr(roblox, "RobloxVoiceSafety", lambda: worker)
    result = asyncio.run(MODEL_SUBMITTERS[roblox.PUBLIC_MODEL_ID]("https://example.com/audio", "transcript"))
    assert result is handle
    spawn.assert_awaited_once_with("https://example.com/audio", "transcript")
    handle.get.assert_not_called()


def test_registry_import_does_not_load_gpu_dependencies() -> None:
    subprocess.run(
        [sys.executable, "-c", (
            "import sys; "
            "from socialguard_models.moderation.models.registry import MODEL_SUBMITTERS; "
            "assert 'roblox-voice-safety-v3' in MODEL_SUBMITTERS; "
            "assert not {'torch', 'numpy', 'safetensors', 'huggingface_hub', "
            "'_socialguard_roblox_voice_safety_v3'} & sys.modules.keys()"
        )],
        check=True,
    )
