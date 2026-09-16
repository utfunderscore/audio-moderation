"""Pinned audio-only Roblox Voice Safety v3 runtime on Modal."""

from __future__ import annotations

import importlib.util
import logging
import sys
import wave
from collections.abc import Callable, Iterator, Sequence
from functools import cache
from pathlib import Path
from tempfile import TemporaryDirectory
from time import perf_counter
from typing import Protocol, cast

import modal

from socialguard_models.audio import download_audio, normalize_audio, validate_wav
from socialguard_models.modal_app import app
from socialguard_models.model_job import SubmittedModel
from socialguard_models.moderation.contracts import ModerationScores
from socialguard_models.moderation.models.roblox_voice_safety_audio import (
    audio_windows,
    pad_pcm_window,
)
from socialguard_models.moderation.models.roblox_voice_safety_scores import (
    aggregate_scores,
    map_scores,
    validate_labels,
)

logger = logging.getLogger(__name__)
PUBLIC_MODEL_ID = "roblox-voice-safety-v3"
MODEL_ID = "Roblox/voice-safety-classifier-v3"
MODEL_REVISION = "ddb1ffb03f2a53236b6d61dd846bfc286b1d4889"
MODEL_CACHE_PATH = "/model-cache"
REFERENCE_PATH = "/opt/roblox-voice-safety-v3"
MAX_AUDIO_BYTES = 512 * 1024 * 1024
MAX_AUDIO_SECONDS = 300


class _Tensor(Protocol):
    @property
    def shape(self) -> Sequence[int]: ...

    def float(self) -> _Tensor: ...

    def unsqueeze(self, dim: int) -> _Tensor: ...

    def __truediv__(self, divisor: float) -> _Tensor: ...

    def __getitem__(self, index: int) -> _Tensor: ...

    def tolist(self) -> list[object]: ...


class _Reference(Protocol):
    def load_model(self, model_dir: str, device: str) -> tuple[object, dict[str, object]]: ...

    def run_inference(self, model: object, audio: _Tensor) -> dict[str, _Tensor]: ...


@cache
def _reference_module() -> _Reference:
    """Import reviewed code baked into the image, never mutable code from the cache."""
    name = "_socialguard_roblox_voice_safety_v3"
    spec = importlib.util.spec_from_file_location(name, Path(REFERENCE_PATH, "inference.py"))
    if spec is None or spec.loader is None:
        raise RuntimeError("Roblox reference inference module is unavailable")
    module = importlib.util.module_from_spec(spec)
    # dataclasses resolve the defining module through sys.modules.
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return cast(_Reference, cast(object, module))


model_cache = modal.Volume.from_name("socialguard-moderation-models", create_if_missing=True)

roblox_image = (
    modal.Image.debian_slim(python_version="3.12")
    .apt_install("ffmpeg")
    .pip_install(
        "torch==2.9.1",
        "numpy==2.2.6",
        "safetensors==0.6.2",
        "huggingface_hub==0.36.0",
    )
    # Pin the reference implementation and its Apache license to the checkpoint.
    # Code lives in the immutable image; the volume holds only weights/config.
    .run_commands(
        "python -c \"from huggingface_hub import snapshot_download; "
        f"snapshot_download('{MODEL_ID}', revision='{MODEL_REVISION}', "
        f"local_dir='{REFERENCE_PATH}', allow_patterns=['inference.py', 'LICENSE.md'])\""
    )
    .env({"HF_HOME": MODEL_CACHE_PATH})
    .add_local_python_source("socialguard_models")
)


def _download_model() -> str:
    from huggingface_hub import snapshot_download  # pyright: ignore[reportMissingImports, reportUnknownVariableType]

    return cast(Callable[..., str], snapshot_download)(
        repo_id=MODEL_ID,
        revision=MODEL_REVISION,
        cache_dir=MODEL_CACHE_PATH,
        allow_patterns=["config.json", "model.safetensors"],
    )


@app.function(image=roblox_image)  # pyright: ignore[reportUnknownMemberType]
def verify_roblox_image_imports() -> str:
    """Verify the custom architecture imports without a GPU or checkpoint download."""
    _reference_module()
    return f"Roblox Voice Safety v3 reference imports succeeded: {MODEL_REVISION}"


@app.function(  # pyright: ignore[reportUnknownMemberType]
    image=roblox_image,
    timeout=1_800,
    volumes={MODEL_CACHE_PATH: model_cache},
)
def download_roblox_model() -> str:
    """Preload the exact checkpoint consumed by inference containers."""
    model_path = _download_model()
    model_cache.commit()
    return model_path


@app.cls(  # pyright: ignore[reportUnknownMemberType]
    image=roblox_image,
    gpu="L4",
    memory=16_384,
    scaledown_window=300,
    timeout=660,
    volumes={MODEL_CACHE_PATH: model_cache},
)
class RobloxVoiceSafety:
    """One recording at a time, with sequential overlapping-window inference."""

    @modal.enter()  # pyright: ignore[reportUnknownMemberType]
    def load_model(self) -> None:
        import torch  # pyright: ignore[reportMissingImports]

        if not cast(Callable[[], bool], torch.cuda.is_available)():  # pyright: ignore[reportUnknownMemberType]
            raise RuntimeError("Roblox Voice Safety requires a CUDA GPU")
        self._reference = _reference_module()
        self._model, config = self._reference.load_model(_download_model(), device="cuda")
        self._labels = validate_labels(config.get("labels"))
        if (
            config.get("sample_rate") != 16_000
            or config.get("num_labels") != 8
            or config.get("max_positions") != 1_500
            or config.get("hop_length") != 160
        ):
            raise ValueError("unexpected Roblox audio configuration")
        model_cache.commit()

    @modal.method()  # pyright: ignore[reportUnknownMemberType]
    def moderate(self, audio_url: str, transcription: str) -> ModerationScores:
        """Accept the family contract; this classifier uses audio only."""
        with TemporaryDirectory() as directory:
            source = Path(directory, "source-audio")
            download_audio(audio_url, source, max_bytes=MAX_AUDIO_BYTES)
            return self._moderate_file(source)

    @modal.method()  # pyright: ignore[reportUnknownMemberType]
    def moderate_bytes(self, audio: bytes, transcription: str = "") -> ModerationScores:
        """Developer entrypoint using the same normalization and inference path."""
        if len(audio) > MAX_AUDIO_BYTES:
            raise ValueError("audio file exceeds the 512 MiB limit")
        with TemporaryDirectory() as directory:
            source = Path(directory, "source-audio")
            source.write_bytes(audio)
            return self._moderate_file(source)

    def _moderate_file(self, source: Path) -> ModerationScores:
        started = perf_counter()
        with TemporaryDirectory() as directory:
            normalized = Path(directory, "audio.wav")
            normalize_audio(source, normalized, max_seconds=MAX_AUDIO_SECONDS)
            sample_count = validate_wav(normalized, max_seconds=MAX_AUDIO_SECONDS)
            windows = list(audio_windows(sample_count))

            def infer_windows() -> Iterator[ModerationScores]:
                with wave.open(str(normalized), "rb") as wav_file:
                    for start, end in windows:
                        wav_file.setpos(start)
                        pcm = wav_file.readframes(end - start)
                        if len(pcm) != (end - start) * 2:
                            raise ValueError("truncated normalized audio")
                        yield self._infer_window(pad_pcm_window(pcm))

            result = aggregate_scores(infer_windows())
        logger.info(
            "Roblox moderation completed: revision=%s samples=%d windows=%d elapsed_seconds=%.3f",
            MODEL_REVISION, sample_count, len(windows), perf_counter() - started,
        )
        return result

    def _infer_window(self, pcm: bytes) -> ModerationScores:
        import torch  # pyright: ignore[reportMissingImports]

        waveform = cast(Callable[..., _Tensor], torch.frombuffer)(  # pyright: ignore[reportUnknownMemberType]
            bytearray(pcm), dtype=torch.int16,  # pyright: ignore[reportUnknownMemberType]
        ).float() / 32768.0
        raw = self._reference.run_inference(self._model, waveform.unsqueeze(0))
        probabilities = raw["probs"]
        if tuple(probabilities.shape) != (1, len(self._labels)):
            raise ValueError("unexpected Roblox probability tensor shape")
        return map_scores(self._labels, probabilities[0].tolist())


async def submit_roblox_voice_safety(
    audio_url: str, transcription: str,
) -> SubmittedModel[ModerationScores]:
    return cast(
        SubmittedModel[ModerationScores],
        await RobloxVoiceSafety().moderate.spawn.aio(audio_url, transcription),  # pyright: ignore[reportAttributeAccessIssue, reportUnknownMemberType]
    )
