"""Modal worker for Roblox voice-safety classifier v3."""

import importlib.util
import json
import sys
from collections.abc import Iterator
from functools import cache
from http.client import HTTPMessage
from pathlib import Path
from shutil import copyfileobj
from tempfile import TemporaryDirectory
from typing import IO, NotRequired, Protocol, TypedDict, cast, override
from urllib.error import HTTPError, URLError
from urllib.request import HTTPRedirectHandler, Request, build_opener

import modal

from socialguard_models.api.networking import UnsafeUrlError, validate_url
from socialguard_models.voice_safety.models import (
    VoiceSafetyJob,
    VoiceSafetyResult,
    VoiceSafetyScores,
)
from socialguard_models.voice_safety.models.roblox import MODEL_ID, MODEL_REVISION
from socialguard_models.voice_safety.models.roblox.scores import normalize_scores

app = modal.App("socialguard-voice-safety")
MODEL_DIRECTORY = Path("/models/voice-safety-classifier-v3") / MODEL_REVISION
MODEL_READY_MARKER = MODEL_DIRECTORY / ".ready"
MODEL_ALLOW_PATTERNS = ["config.json", "inference.py", "model.safetensors"]
DOWNLOAD_TIMEOUT_SECONDS = 30
DOWNLOAD_CHUNK_SIZE = 1024 * 1024
SAMPLE_RATE = 16_000
CHUNK_SECONDS = 15
CHUNK_OVERLAP_SECONDS = 3
MAX_AUDIO_SECONDS = 10 * 60
MODEL_DOWNLOAD_COMMAND = (
    "uv run modal run "
    "src/socialguard_models/voice_safety/models/roblox/roblox_model.py"
    "::download_roblox_model"
)
MODEL_NOT_READY_MESSAGE = (
    "Roblox voice-safety model weights are missing. Run "
    f"`{MODEL_DOWNLOAD_COMMAND}` before using or deploying the worker."
)
MODEL_CONFIG_ERROR = "Roblox voice-safety config must be a JSON object"
DOWNLOAD_AUDIO_ERROR = "Unable to download audio object"
INVALID_AUDIO_FORMAT_ERROR = "Audio must be a 16 kHz PCM WAV"
EMPTY_AUDIO_ERROR = "Audio object contains no samples"
AUDIO_TOO_LONG_ERROR = "Audio must not exceed 10 minutes"
AUDIO_TRUNCATED_ERROR = "Audio ended before its declared sample count"
LANGUAGE_HEADS_CHANGED_ERROR = "Language probability heads changed between audio chunks"
INFERENCE_LOAD_ERROR = "Unable to load Roblox voice-safety inference module"


class _Audio(Protocol):
    @property
    def shape(self) -> tuple[int, ...]: ...


class _RawInference(TypedDict):
    probs: object
    language_probs: NotRequired[object | None]


class _ExtractedScores(TypedDict):
    label_scores: dict[str, float]
    language_probs: NotRequired[dict[str, float]]


class _InferenceModule(Protocol):
    def load_model(
        self, model_dir: Path, *, device: str
    ) -> tuple[object, dict[str, object]]: ...

    def run_inference(
        self, model: object, audio: _Audio, *, device: str
    ) -> _RawInference: ...

    def extract_label_scores(
        self,
        probs: object,
        language_probs: object | None,
        config: dict[str, object],
        *,
        index: int,
    ) -> _ExtractedScores: ...


roblox_image = (
    modal.Image.debian_slim(python_version="3.12")
    .apt_install("libsndfile1")
    .pip_install(
        "numpy==2.3.3",
        "safetensors==0.6.2",
        "soundfile==0.13.1",
        "torch==2.8.0",
        "pydantic>=2,<3",
    )
    .add_local_python_source("socialguard_models")
)

model_download_image = (
    modal.Image.debian_slim(python_version="3.12")
    .pip_install(
        "huggingface_hub==0.36.2",
        "pydantic>=2,<3",
    )
    .add_local_python_source("socialguard_models")
)
model_volume = modal.Volume.from_name(
    "socialguard-model-weights", create_if_missing=True
)


@app.function(  # pyright: ignore[reportUnknownMemberType]
    image=model_download_image,
    volumes={"/models": model_volume},
    max_containers=1,
)
def download_roblox_model() -> None:
    """Download the pinned model and inference code into the persistent Volume."""
    from huggingface_hub import (  # pyright: ignore[reportMissingModuleSource]
        snapshot_download,
    )

    if MODEL_READY_MARKER.exists():
        return

    MODEL_DIRECTORY.mkdir(parents=True, exist_ok=True)
    snapshot_download(
        repo_id=MODEL_ID,
        revision=MODEL_REVISION,
        local_dir=MODEL_DIRECTORY,
        allow_patterns=MODEL_ALLOW_PATTERNS,
    )
    MODEL_READY_MARKER.touch()
    model_volume.commit()


def _ensure_model_ready() -> None:
    if not MODEL_READY_MARKER.exists():
        model_volume.reload()
    if not MODEL_READY_MARKER.exists():
        raise RuntimeError(MODEL_NOT_READY_MESSAGE)


@cache
def _load_inference_module() -> _InferenceModule:
    """Load the inference implementation from the immutable model snapshot."""
    _ensure_model_ready()
    module_name = "_socialguard_roblox_voice_safety_inference"
    spec = importlib.util.spec_from_file_location(
        module_name, MODEL_DIRECTORY / "inference.py"
    )
    if spec is None or spec.loader is None:
        raise RuntimeError(INFERENCE_LOAD_ERROR)

    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    try:
        spec.loader.exec_module(module)
    except Exception:
        _ = sys.modules.pop(module_name, None)
        raise
    return cast("_InferenceModule", cast("object", module))


@cache
def _load_config() -> dict[str, object]:
    _ensure_model_ready()
    with (MODEL_DIRECTORY / "config.json").open(encoding="utf-8") as config_file:
        config = cast("object", json.load(config_file))
    if not isinstance(config, dict):
        raise TypeError(MODEL_CONFIG_ERROR)
    return cast("dict[str, object]", config)


@cache
def _load_model() -> object:
    """Keep the GPU model alive between jobs in a warm container."""
    inference = _load_inference_module()
    model, _ = inference.load_model(MODEL_DIRECTORY, device="cuda")
    return model


class _ValidatedRedirectHandler(HTTPRedirectHandler):
    """Validate every redirect before urllib follows it."""

    @override
    def redirect_request(
        self,
        req: Request,
        fp: IO[bytes],
        code: int,
        msg: str,
        headers: HTTPMessage,
        newurl: str,
    ) -> Request | None:
        """Reject redirects that violate the standard network policy."""
        validate_url(newurl)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def _download_audio(download_url: str, destination: Path) -> None:
    """Stream an internally generated presigned URL into a temporary file."""
    try:
        validate_url(download_url)
        opener = build_opener(_ValidatedRedirectHandler())
        with (
            opener.open(download_url, timeout=DOWNLOAD_TIMEOUT_SECONDS) as response,
            destination.open("wb") as audio,
        ):
            copyfileobj(response, audio, length=DOWNLOAD_CHUNK_SIZE)
    except HTTPError as error:
        error.close()
        raise RuntimeError(DOWNLOAD_AUDIO_ERROR) from None
    except (URLError, UnsafeUrlError, TimeoutError, OSError):
        raise RuntimeError(DOWNLOAD_AUDIO_ERROR) from None


def _chunk_ranges(sample_count: int) -> list[tuple[int, int]]:
    """Return 15-second windows whose starts are 12 seconds apart."""
    chunk_samples = SAMPLE_RATE * CHUNK_SECONDS
    stride_samples = SAMPLE_RATE * (CHUNK_SECONDS - CHUNK_OVERLAP_SECONDS)
    ranges: list[tuple[int, int]] = []
    start = 0
    while start < sample_count:
        end = min(start + chunk_samples, sample_count)
        ranges.append((start, end))
        if end == sample_count:
            break
        start += stride_samples
    return ranges


def _validate_audio(audio_path: Path) -> int:
    """Validate the WAV metadata before loading the GPU model."""
    import soundfile as sf  # pyright: ignore[reportMissingModuleSource]

    with sf.SoundFile(audio_path) as recording:
        if (
            recording.samplerate != SAMPLE_RATE
            or recording.channels < 1
            or recording.format not in {"WAV", "WAVEX"}
            or not recording.subtype.startswith("PCM_")
        ):
            raise ValueError(INVALID_AUDIO_FORMAT_ERROR)
        sample_count = len(recording)

    if sample_count == 0:
        raise ValueError(EMPTY_AUDIO_ERROR)
    if sample_count > SAMPLE_RATE * MAX_AUDIO_SECONDS:
        raise ValueError(AUDIO_TOO_LONG_ERROR)
    return sample_count


def _load_audio_chunks(
    audio_path: Path, ranges: list[tuple[int, int]]
) -> Iterator[_Audio]:
    """Read one first-channel float32 window at a time from the validated WAV."""
    import soundfile as sf  # pyright: ignore[reportMissingModuleSource]
    import torch  # pyright: ignore[reportMissingModuleSource]

    with sf.SoundFile(audio_path) as recording:
        for start, end in ranges:
            recording.seek(start)
            samples = recording.read(
                frames=end - start,
                dtype="float32",
                always_2d=True,
            )
            if samples.shape[0] != end - start:
                raise ValueError(AUDIO_TRUNCATED_ERROR)
            yield torch.from_numpy(samples[:, 0].copy()).unsqueeze(0)


@app.function(  # pyright: ignore[reportUnknownMemberType]
    image=roblox_image,
    gpu="L4",
    volumes={"/models": model_volume.with_mount_options(read_only=True)},
    timeout=5 * 60,
    scaledown_window=2,
    min_containers=0,
    buffer_containers=0,
    max_containers=1,
)
def classify_roblox(job: VoiceSafetyJob) -> VoiceSafetyResult:
    """Classify up to 10 minutes using overlapping 15-second windows."""
    with TemporaryDirectory() as directory:
        audio_path = Path(directory) / "audio.wav"
        _download_audio(job.download_url, audio_path)

        ranges = _chunk_ranges(_validate_audio(audio_path))
        inference = _load_inference_module()
        config = _load_config()
        model = _load_model()
        score_maxima: dict[str, float] = {}
        language_totals: dict[str, float] = {}
        language_sample_count = 0

        for audio in _load_audio_chunks(audio_path, ranges):
            raw = inference.run_inference(model, audio, device="cuda")
            extracted = inference.extract_label_scores(
                raw["probs"],
                raw.get("language_probs"),
                config,
                index=0,
            )
            chunk_result = VoiceSafetyResult(
                scores=normalize_scores(extracted["label_scores"]),
                language_probs=extracted.get("language_probs", {}),
            )

            chunk_scores = cast("dict[str, float]", chunk_result.scores.model_dump())
            for category, score in chunk_scores.items():
                score_maxima[category] = max(score_maxima.get(category, 0.0), score)

            if chunk_result.language_probs:
                if (
                    language_totals
                    and language_totals.keys() != chunk_result.language_probs.keys()
                ):
                    raise RuntimeError(LANGUAGE_HEADS_CHANGED_ERROR)
                chunk_samples = audio.shape[-1]
                for language, probability in chunk_result.language_probs.items():
                    language_totals[language] = (
                        language_totals.get(language, 0.0) + probability * chunk_samples
                    )
                language_sample_count += chunk_samples

    return VoiceSafetyResult(
        scores=VoiceSafetyScores.model_validate(score_maxima),
        language_probs={
            language: total / language_sample_count
            for language, total in language_totals.items()
        },
    )
