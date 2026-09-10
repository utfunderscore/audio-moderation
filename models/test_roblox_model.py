"""Focused tests for the Roblox voice-safety worker."""

from email.message import Message
from io import BytesIO
from pathlib import Path
import sys
from tempfile import TemporaryDirectory as LocalTemporaryDirectory
import unittest
from types import SimpleNamespace
from typing import Any, cast
from urllib.error import HTTPError
from unittest.mock import Mock, call, patch

from socialguard_models.voice_safety.models import VoiceSafetyJob
from socialguard_models.voice_safety.models.roblox import MODEL_ID, MODEL_REVISION, roblox_model


class TemporaryDirectory:
    def __init__(self) -> None:
        self.cleaned_up = False

    def __enter__(self) -> str:
        return "/temporary/roblox"

    def __exit__(self, *args: object) -> None:
        self.cleaned_up = True


def classify(job: VoiceSafetyJob) -> Any:
    return cast(Any, roblox_model.classify_roblox).local(job)


def label_scores(**overrides: float) -> dict[str, float]:
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


class RobloxModelTests(unittest.TestCase):
    def test_downloads_pinned_snapshot_and_marks_volume_ready(self) -> None:
        snapshot_download = Mock()
        volume = Mock()

        with LocalTemporaryDirectory() as directory:
            model_directory = Path(directory) / "roblox" / MODEL_REVISION
            marker = model_directory / ".ready"
            with (
                patch.dict(
                    sys.modules,
                    {"huggingface_hub": SimpleNamespace(snapshot_download=snapshot_download)},
                ),
                patch.object(roblox_model, "MODEL_DIRECTORY", model_directory),
                patch.object(roblox_model, "MODEL_READY_MARKER", marker),
                patch.object(roblox_model, "model_volume", volume),
            ):
                cast(Any, roblox_model.download_roblox_model).local()

            snapshot_download.assert_called_once_with(
                repo_id=MODEL_ID,
                revision=MODEL_REVISION,
                local_dir=model_directory,
                allow_patterns=["config.json", "inference.py", "model.safetensors"],
            )
            self.assertTrue(marker.exists())
            volume.commit.assert_called_once_with()

    def test_failed_download_does_not_mark_volume_ready(self) -> None:
        snapshot_download = Mock(side_effect=RuntimeError("download failed"))
        volume = Mock()

        with LocalTemporaryDirectory() as directory:
            model_directory = Path(directory) / "roblox" / MODEL_REVISION
            marker = model_directory / ".ready"
            with (
                patch.dict(
                    sys.modules,
                    {"huggingface_hub": SimpleNamespace(snapshot_download=snapshot_download)},
                ),
                patch.object(roblox_model, "MODEL_DIRECTORY", model_directory),
                patch.object(roblox_model, "MODEL_READY_MARKER", marker),
                patch.object(roblox_model, "model_volume", volume),
            ):
                with self.assertRaisesRegex(RuntimeError, "download failed"):
                    cast(Any, roblox_model.download_roblox_model).local()

            self.assertFalse(marker.exists())
            volume.commit.assert_not_called()

    def test_loads_and_caches_model_on_cuda(self) -> None:
        model = Mock()
        inference = SimpleNamespace(load_model=Mock(return_value=(model, {})))
        roblox_model._load_model.cache_clear()
        try:
            with (
                patch.object(roblox_model, "_load_inference_module", return_value=inference),
                patch.object(roblox_model, "MODEL_DIRECTORY", Path("/models/test")),
            ):
                first = roblox_model._load_model()
                second = roblox_model._load_model()
        finally:
            roblox_model._load_model.cache_clear()

        self.assertIs(first, second)
        inference.load_model.assert_called_once_with(Path("/models/test"), device="cuda")

    def test_missing_model_reloads_volume_then_shows_setup_command(self) -> None:
        volume = Mock()

        with LocalTemporaryDirectory() as directory:
            model_directory = Path(directory) / "roblox" / MODEL_REVISION
            marker = model_directory / ".ready"
            with (
                patch.object(roblox_model, "MODEL_DIRECTORY", model_directory),
                patch.object(roblox_model, "MODEL_READY_MARKER", marker),
                patch.object(roblox_model, "model_volume", volume),
            ):
                with self.assertRaisesRegex(RuntimeError, "modal run .*download_roblox_model"):
                    roblox_model._ensure_model_ready()

        volume.reload.assert_called_once_with()

    def test_classifies_audio_and_normalizes_result(self) -> None:
        first_audio = SimpleNamespace(shape=(1, 240_000))
        second_audio = SimpleNamespace(shape=(1, 48_000))
        first_raw = {"probs": object(), "language_probs": object()}
        second_raw = {"probs": object(), "language_probs": object()}
        inference = SimpleNamespace(
            run_inference=Mock(side_effect=[first_raw, second_raw]),
            extract_label_scores=Mock(
                side_effect=[
                    {
                        "label_scores": label_scores(),
                        "language_probs": {"en": 0.75, "es": 0.25},
                    },
                    {
                        "label_scores": label_scores(
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
        directory = TemporaryDirectory()
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
            result = classify(
                VoiceSafetyJob(download_url="https://s3.example/audio.wav?secret")
            )

        audio_path = Path("/temporary/roblox/audio.wav")
        download_audio.assert_called_once_with(
            "https://s3.example/audio.wav?secret", audio_path
        )
        load_audio_chunks.assert_called_once_with(audio_path, ranges)
        self.assertEqual(
            inference.run_inference.call_args_list,
            [
                call(model, first_audio, device="cuda"),
                call(model, second_audio, device="cuda"),
            ],
        )
        self.assertEqual(
            inference.extract_label_scores.call_args_list,
            [
                call(
                    first_raw["probs"], first_raw["language_probs"], config, index=0
                ),
                call(
                    second_raw["probs"], second_raw["language_probs"], config, index=0
                ),
            ],
        )
        self.assertEqual(result.scores.sexual, 0.9)
        self.assertEqual(result.scores.harassment_or_abuse, 0.7)
        self.assertAlmostEqual(result.language_probs["en"], 2 / 3)
        self.assertAlmostEqual(result.language_probs["es"], 1 / 3)
        self.assertTrue(directory.cleaned_up)

    def test_rejects_empty_audio_before_loading_model(self) -> None:
        load_model = Mock()

        with (
            patch.object(roblox_model, "TemporaryDirectory", return_value=TemporaryDirectory()),
            patch.object(roblox_model, "_download_audio"),
            patch.object(
                roblox_model,
                "_validate_audio",
                side_effect=ValueError("Audio object contains no samples"),
            ),
            patch.object(roblox_model, "_load_model", load_model),
        ):
            with self.assertRaisesRegex(ValueError, "contains no samples"):
                classify(VoiceSafetyJob(download_url="https://s3.example/empty.wav?secret"))

        load_model.assert_not_called()

    def test_builds_fifteen_second_ranges_with_three_second_overlap(self) -> None:
        second = roblox_model.SAMPLE_RATE
        cases = [
            (1, [(0, 1)]),
            (15 * second, [(0, 15 * second)]),
            (
                15 * second + 1,
                [(0, 15 * second), (12 * second, 15 * second + 1)],
            ),
            (
                27 * second,
                [(0, 15 * second), (12 * second, 27 * second)],
            ),
        ]
        for sample_count, expected in cases:
            with self.subTest(sample_count=sample_count):
                self.assertEqual(roblox_model._chunk_ranges(sample_count), expected)

    def test_validates_pcm_wav_duration_before_model_loading(self) -> None:
        class Recording:
            samplerate = 16_000
            channels = 1
            format = "WAV"
            subtype = "PCM_16"

            def __init__(self, sample_count: int) -> None:
                self.sample_count = sample_count

            def __enter__(self) -> "Recording":
                return self

            def __exit__(self, *args: object) -> None:
                pass

            def __len__(self) -> int:
                return self.sample_count

        recording = Recording(roblox_model.SAMPLE_RATE * roblox_model.MAX_AUDIO_SECONDS)
        soundfile = SimpleNamespace(SoundFile=Mock(return_value=recording))
        with patch.dict(sys.modules, {"soundfile": soundfile}):
            self.assertEqual(
                roblox_model._validate_audio(Path("audio.wav")),
                len(recording),
            )

        recording.sample_count += 1
        with patch.dict(sys.modules, {"soundfile": soundfile}):
            with self.assertRaisesRegex(ValueError, "10 minutes"):
                roblox_model._validate_audio(Path("audio.wav"))

        recording.sample_count = 1
        recording.samplerate = 48_000
        with patch.dict(sys.modules, {"soundfile": soundfile}):
            with self.assertRaisesRegex(ValueError, "16 kHz PCM WAV"):
                roblox_model._validate_audio(Path("audio.wav"))

    def test_download_url_is_hidden_and_http_failure_is_sanitized(self) -> None:
        download_url = "https://s3.example/audio.wav?X-Amz-Signature=bearer-secret"
        job = VoiceSafetyJob(download_url=download_url)
        self.assertNotIn(download_url, repr(job))

        error_body = BytesIO()
        error = HTTPError(download_url, 403, "Forbidden", Message(), error_body)  # type: ignore[arg-type]
        with patch.object(roblox_model, "urlopen", side_effect=error):
            with self.assertRaisesRegex(RuntimeError, "Unable to download audio object") as raised:
                roblox_model._download_audio(download_url, Path("unused"))

        self.assertNotIn(download_url, str(raised.exception))
        self.assertIsNone(raised.exception.__cause__)
        self.assertTrue(error_body.closed)


if __name__ == "__main__":
    unittest.main()
