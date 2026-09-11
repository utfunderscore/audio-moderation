"""Tests for the Roblox voice-safety worker."""

import sys
from http.client import HTTPMessage
from io import BytesIO
from pathlib import Path
from tempfile import TemporaryDirectory as LocalTemporaryDirectory
from types import SimpleNamespace
from typing import Never, Self, cast
from unittest.mock import Mock, call, patch
from urllib.error import HTTPError
from urllib.parse import SplitResult, urlsplit
from urllib.request import Request

import pytest

from socialguard_models.api.networking import UnsafeUrlError
from socialguard_models.voice_safety.models import (
    VoiceSafetyJob,
    VoiceSafetyResult,
    VoiceSafetyWorker,
)
from socialguard_models.voice_safety.models.roblox import (
    MODEL_ID,
    MODEL_REVISION,
    roblox_model,
)


class _TemporaryDirectory:
    """Track cleanup without creating an inference fixture on disk."""

    def __init__(self) -> None:
        """Initialize an uncleaned temporary directory."""
        self.cleaned_up = False

    def __enter__(self) -> str:
        """Return a stable fixture path."""
        return "/temporary/roblox"

    def __exit__(self, *args: object) -> None:
        """Record context-manager cleanup."""
        self.cleaned_up = True


class _Recording:
    """Minimal SoundFile-compatible recording fixture."""

    samplerate = 16_000
    channels = 1
    format = "WAV"
    subtype = "PCM_16"

    def __init__(self, sample_count: int) -> None:
        """Set the number of available audio samples."""
        self.sample_count = sample_count

    def __enter__(self) -> Self:
        """Enter the recording context."""
        return self

    def __exit__(self, *args: object) -> None:
        """Close the fixture recording."""

    def __len__(self) -> int:
        """Return the declared sample count."""
        return self.sample_count


def _classify(job: VoiceSafetyJob) -> VoiceSafetyResult:
    worker = cast("VoiceSafetyWorker", roblox_model.classify_roblox)
    return worker.local(job)


def _download_model() -> None:
    roblox_model.download_roblox_model.local()


def _label_scores(**overrides: float) -> dict[str, float]:
    scores = {
        "ABUSE_TYPE_PRIVACY_ASKING_FOR_PII": 0.1,
        "ABUSE_TYPE_DISCRIMINATORY": 0.2,
        "ABUSE_TYPE_HARASSMENT": 0.3,
        "ABUSE_TYPE_SEXUAL_CONTENT": 0.4,
        "ABUSE_TYPE_ILLEGAL_AND_REGULATED_CONTENT": 0.5,
        "ABUSE_TYPE_DATING_AND_ROMANTIC_CONTENT": 0.6,
        "ABUSE_TYPE_PROFANITY": 0.7,
    }
    scores.update(overrides)
    return scores


def test_downloads_pinned_snapshot_and_marks_volume_ready(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """The model download pins its revision and commits only after completion."""
    snapshot_download = Mock()
    volume = Mock()

    with LocalTemporaryDirectory() as directory:
        model_directory = Path(directory) / "roblox" / MODEL_REVISION
        marker = model_directory / ".ready"
        monkeypatch.setitem(
            sys.modules,
            "huggingface_hub",
            SimpleNamespace(snapshot_download=snapshot_download),
        )
        with (
            patch.object(roblox_model, "MODEL_DIRECTORY", model_directory),
            patch.object(roblox_model, "MODEL_READY_MARKER", marker),
            patch.object(roblox_model, "model_volume", volume),
        ):
            _download_model()

        snapshot_download.assert_called_once_with(
            repo_id=MODEL_ID,
            revision=MODEL_REVISION,
            local_dir=model_directory,
            allow_patterns=["config.json", "inference.py", "model.safetensors"],
        )
        assert marker.exists()
        volume.commit.assert_called_once_with()


def test_failed_download_does_not_mark_volume_ready(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """A failed snapshot download leaves the model unavailable."""
    snapshot_download = Mock(side_effect=RuntimeError("download failed"))
    volume = Mock()

    with LocalTemporaryDirectory() as directory:
        model_directory = Path(directory) / "roblox" / MODEL_REVISION
        marker = model_directory / ".ready"
        monkeypatch.setitem(
            sys.modules,
            "huggingface_hub",
            SimpleNamespace(snapshot_download=snapshot_download),
        )
        with (
            patch.object(roblox_model, "MODEL_DIRECTORY", model_directory),
            patch.object(roblox_model, "MODEL_READY_MARKER", marker),
            patch.object(roblox_model, "model_volume", volume),
            pytest.raises(RuntimeError, match="download failed"),
        ):
            _download_model()

        assert not marker.exists()
        volume.commit.assert_not_called()


def test_loads_and_caches_model_on_cuda() -> None:
    """A warm container reuses one CUDA-loaded model."""
    model = Mock()
    inference = SimpleNamespace(load_model=Mock(return_value=(model, {})))
    roblox_model._load_model.cache_clear()  # pyright: ignore[reportPrivateUsage]
    try:
        with (
            patch.object(
                roblox_model, "_load_inference_module", return_value=inference
            ),
            patch.object(roblox_model, "MODEL_DIRECTORY", Path("/models/test")),
        ):
            first = roblox_model._load_model()  # pyright: ignore[reportPrivateUsage]
            second = roblox_model._load_model()  # pyright: ignore[reportPrivateUsage]
    finally:
        roblox_model._load_model.cache_clear()  # pyright: ignore[reportPrivateUsage]

    assert first is second
    inference.load_model.assert_called_once_with(Path("/models/test"), device="cuda")


def test_missing_model_reloads_volume_then_shows_setup_command() -> None:
    """A missing model reports the exact setup command after reloading the Volume."""
    volume = Mock()

    with LocalTemporaryDirectory() as directory:
        model_directory = Path(directory) / "roblox" / MODEL_REVISION
        marker = model_directory / ".ready"
        with (
            patch.object(roblox_model, "MODEL_DIRECTORY", model_directory),
            patch.object(roblox_model, "MODEL_READY_MARKER", marker),
            patch.object(roblox_model, "model_volume", volume),
            pytest.raises(RuntimeError, match=r"modal run .*download_roblox_model"),
        ):
            roblox_model._ensure_model_ready()  # pyright: ignore[reportPrivateUsage]

    volume.reload.assert_called_once_with()


def test_classifies_audio_and_normalizes_result() -> None:
    """Chunk maxima and sample-weighted language probabilities are returned."""
    first_audio = SimpleNamespace(shape=(1, 240_000))
    second_audio = SimpleNamespace(shape=(1, 48_000))
    first_raw = {"probs": object(), "language_probs": object()}
    second_raw = {"probs": object(), "language_probs": object()}
    inference = SimpleNamespace(
        run_inference=Mock(side_effect=[first_raw, second_raw]),
        extract_label_scores=Mock(
            side_effect=[
                {
                    "label_scores": _label_scores(),
                    "language_probs": {"en": 0.75, "es": 0.25},
                },
                {
                    "label_scores": _label_scores(
                        ABUSE_TYPE_SEXUAL_CONTENT=0.9,
                        ABUSE_TYPE_PROFANITY=0.2,
                    ),
                    "language_probs": {"en": 0.25, "es": 0.75},
                },
            ]
        ),
    )
    model = object()
    config = {"sample_rate": 16_000}
    directory = _TemporaryDirectory()
    ranges = [(0, 240_000), (192_000, 240_001)]

    with (
        patch.object(roblox_model, "TemporaryDirectory", return_value=directory),
        patch.object(roblox_model, "_download_audio") as download_audio,
        patch.object(roblox_model, "_validate_audio", return_value=240_001),
        patch.object(
            roblox_model,
            "_load_audio_chunks",
            return_value=iter([first_audio, second_audio]),
        ) as load_audio_chunks,
        patch.object(roblox_model, "_load_inference_module", return_value=inference),
        patch.object(roblox_model, "_load_config", return_value=config),
        patch.object(roblox_model, "_load_model", return_value=model),
    ):
        result = _classify(
            VoiceSafetyJob(download_url="https://s3.example/audio.wav?secret")
        )

    audio_path = Path("/temporary/roblox/audio.wav")
    download_audio.assert_called_once_with(
        "https://s3.example/audio.wav?secret", audio_path
    )
    load_audio_chunks.assert_called_once_with(audio_path, ranges)
    assert inference.run_inference.call_args_list == [
        call(model, first_audio, device="cuda"),
        call(model, second_audio, device="cuda"),
    ]
    assert inference.extract_label_scores.call_args_list == [
        call(first_raw["probs"], first_raw["language_probs"], config, index=0),
        call(second_raw["probs"], second_raw["language_probs"], config, index=0),
    ]
    expected_scores = {"sexual": 0.9, "harassment_or_abuse": 0.7}
    assert result.scores.sexual == expected_scores["sexual"]
    assert result.scores.harassment_or_abuse == expected_scores["harassment_or_abuse"]
    assert result.language_probs["en"] == pytest.approx(2 / 3)
    assert result.language_probs["es"] == pytest.approx(1 / 3)
    assert directory.cleaned_up


def test_rejects_empty_audio_before_loading_model() -> None:
    """Invalid audio fails before model allocation."""
    load_model = Mock()

    with (
        patch.object(
            roblox_model, "TemporaryDirectory", return_value=_TemporaryDirectory()
        ),
        patch.object(roblox_model, "_download_audio"),
        patch.object(
            roblox_model,
            "_validate_audio",
            side_effect=ValueError("Audio object contains no samples"),
        ),
        patch.object(roblox_model, "_load_model", load_model),
        pytest.raises(ValueError, match="contains no samples"),
    ):
        _classify(VoiceSafetyJob(download_url="https://s3.example/empty.wav?secret"))

    load_model.assert_not_called()


def test_builds_fifteen_second_ranges_with_three_second_overlap() -> None:
    """Windows are 15 seconds and advance by 12 seconds."""
    cases = [
        (1, [(0, 1)]),
        (15 * roblox_model.SAMPLE_RATE, [(0, 15 * roblox_model.SAMPLE_RATE)]),
        (
            15 * roblox_model.SAMPLE_RATE + 1,
            [
                (0, 15 * roblox_model.SAMPLE_RATE),
                (12 * roblox_model.SAMPLE_RATE, 15 * roblox_model.SAMPLE_RATE + 1),
            ],
        ),
        (
            27 * roblox_model.SAMPLE_RATE,
            [
                (0, 15 * roblox_model.SAMPLE_RATE),
                (12 * roblox_model.SAMPLE_RATE, 27 * roblox_model.SAMPLE_RATE),
            ],
        ),
    ]
    for sample_count, expected in cases:
        assert roblox_model._chunk_ranges(sample_count) == expected  # pyright: ignore[reportPrivateUsage]


def test_validates_pcm_wav_duration_before_model_loading(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Only bounded 16 kHz PCM WAV metadata is accepted."""
    recording = _Recording(roblox_model.SAMPLE_RATE * roblox_model.MAX_AUDIO_SECONDS)
    soundfile = SimpleNamespace(SoundFile=Mock(return_value=recording))
    monkeypatch.setitem(sys.modules, "soundfile", soundfile)

    assert roblox_model._validate_audio(Path("audio.wav")) == len(recording)  # pyright: ignore[reportPrivateUsage]

    recording.sample_count += 1
    with pytest.raises(ValueError, match="10 minutes"):
        roblox_model._validate_audio(Path("audio.wav"))  # pyright: ignore[reportPrivateUsage]

    recording.sample_count = 1
    recording.samplerate = 48_000
    with pytest.raises(ValueError, match="16 kHz PCM WAV"):
        roblox_model._validate_audio(Path("audio.wav"))  # pyright: ignore[reportPrivateUsage]


def test_rejects_unsafe_urls_and_sanitizes_download_failures(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Unsafe URLs never reach urllib and download errors do not leak signatures."""
    with pytest.raises(UnsafeUrlError):
        roblox_model._ValidatedRedirectHandler().redirect_request(  # pyright: ignore[reportPrivateUsage]
            Request("https://s3.example/audio.wav"),
            BytesIO(),
            302,
            "Found",
            HTTPMessage(),
            "http://s3.example/redirected.wav",
        )

    def unreachable_opener(*handlers: object) -> Never:
        """Fail if validation does not reject the URL first."""
        del handlers
        message = "URL validation should reject before opening"
        raise AssertionError(message)

    monkeypatch.setattr(roblox_model, "build_opener", unreachable_opener)
    with pytest.raises(RuntimeError, match="Unable to download audio object"):
        roblox_model._download_audio("http://s3.example/audio.wav", Path("unused"))  # pyright: ignore[reportPrivateUsage]

    download_url = "https://s3.example/audio.wav?X-Amz-Signature=bearer-secret"
    error_body = BytesIO()
    error = HTTPError(download_url, 403, "Forbidden", HTTPMessage(), error_body)

    class FailingOpener:
        """Return a deterministic HTTP error without opening a network connection."""

        def open(self, url: str, *, timeout: float) -> Never:
            """Raise the configured HTTP failure."""
            del url, timeout
            raise error

    def valid_url(url: str) -> SplitResult:
        """Treat the presigned fixture URL as already network-policy validated."""
        return urlsplit(url)

    def failing_opener(*handlers: object) -> FailingOpener:
        """Return the fixture opener after accepting redirect handlers."""
        del handlers
        return FailingOpener()

    monkeypatch.setattr(roblox_model, "validate_url", valid_url)
    monkeypatch.setattr(roblox_model, "build_opener", failing_opener)
    with pytest.raises(RuntimeError, match="Unable to download audio object") as raised:
        roblox_model._download_audio(download_url, Path("unused"))  # pyright: ignore[reportPrivateUsage]

    assert download_url not in str(raised.value)
    assert raised.value.__cause__ is None
    assert error_body.closed
