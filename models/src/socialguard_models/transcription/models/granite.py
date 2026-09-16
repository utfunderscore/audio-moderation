"""Transcription-only Modal runtime for IBM Granite Speech 4.1 2B."""

from __future__ import annotations

from collections.abc import Callable, Mapping, Sequence
from contextlib import AbstractContextManager
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import Protocol, cast

import modal

from socialguard_models.audio import download_audio, normalize_audio, validate_wav
from socialguard_models.modal_app import app

MODEL_ID = "ibm-granite/granite-speech-4.1-2b"
MODEL_REVISION = "de575db64086f84fdc79da4932d1076e965bc546"
MODEL_CACHE_PATH = "/model-cache"
MAX_AUDIO_BYTES = 512 * 1024 * 1024
MAX_AUDIO_SECONDS = 300
MAX_NEW_TOKENS = 1_024
TRANSCRIPTION_PROMPT = (
    "<|audio|>transcribe the speech with proper punctuation and capitalization."
)


class _Cuda(Protocol):
    def is_available(self) -> bool: ...


class _Torch(Protocol):
    bfloat16: object
    cuda: _Cuda

    def inference_mode(self) -> AbstractContextManager[None]: ...


class _Tensor(Protocol):
    @property
    def shape(self) -> Sequence[int]: ...

    def __getitem__(self, key: object) -> _Tensor: ...

    def unsqueeze(self, dim: int) -> _Tensor: ...


class _Tokenizer(Protocol):
    def apply_chat_template(
        self,
        conversation: list[dict[str, str]],
        *,
        tokenize: bool,
        add_generation_prompt: bool,
    ) -> str: ...

    def batch_decode(
        self,
        sequences: _Tensor,
        *,
        add_special_tokens: bool,
        skip_special_tokens: bool,
    ) -> list[str]: ...


class _Processor(Protocol):
    tokenizer: _Tokenizer

    def __call__(
        self,
        text: str,
        audio: object,
        *,
        device: str,
        return_tensors: str,
    ) -> object: ...


class _Movable(Protocol):
    def to(self, device: str) -> object: ...


class _Model(Protocol):
    def to(self, device: str) -> _Model: ...

    def eval(self) -> object: ...

    def generate(self, **kwargs: object) -> _Tensor: ...


class _AudioArray(Protocol):
    @property
    def shape(self) -> Sequence[int]: ...

    @property
    def T(self) -> object: ...


model_cache = modal.Volume.from_name(
    "socialguard-transcription-models",
    create_if_missing=True,
)

granite_image = (
    modal.Image.debian_slim(python_version="3.12")
    .apt_install("ffmpeg", "libsndfile1")
    .pip_install(
        "huggingface_hub[hf_xet]>=0.34",
        "safetensors>=0.4",
        "soundfile>=0.13",
        "torch==2.9.1",
        "torchaudio==2.9.1",
        "transformers==4.57.6",
    )
    .env(
        {
            "HF_HOME": MODEL_CACHE_PATH,
            "HF_XET_HIGH_PERFORMANCE": "1",
        }
    )
    .add_local_python_source("socialguard_models")
)


@app.function(image=granite_image)  # pyright: ignore[reportUnknownMemberType]
def verify_granite_image_imports() -> str:
    """Import Granite inference dependencies in the configured Modal image."""
    # These dependencies are installed in the GPU image, not the local environment.
    import torch  # pyright: ignore[reportMissingImports]
    import torchaudio  # pyright: ignore[reportMissingImports]
    from transformers import AutoModelForSpeechSeq2Seq, AutoProcessor  # pyright: ignore[reportMissingImports, reportUnknownVariableType]

    return (
        f"torch={torch.__version__}, torchaudio={torchaudio.__version__}; "  # pyright: ignore[reportUnknownMemberType]
        f"{AutoModelForSpeechSeq2Seq.__name__} and {AutoProcessor.__name__} imported"  # pyright: ignore[reportUnknownMemberType]
    )


@app.function(  # pyright: ignore[reportUnknownMemberType]
    image=granite_image,
    timeout=1_800,
    volumes={MODEL_CACHE_PATH: model_cache},
)
def download_granite_model() -> str:
    """Preload the pinned model revision into the shared Modal volume."""
    # These dependencies are installed in the GPU image, not the local environment.
    from huggingface_hub import snapshot_download  # pyright: ignore[reportMissingImports, reportUnknownVariableType]

    model_path = cast(Callable[..., str], snapshot_download)(
        repo_id=MODEL_ID,
        revision=MODEL_REVISION,
        cache_dir=MODEL_CACHE_PATH,
    )
    model_cache.commit()
    return model_path


@app.cls(  # pyright: ignore[reportUnknownMemberType]
    image=granite_image,
    gpu="L4",
    memory=16_384,
    scaledown_window=300,
    timeout=900,
    volumes={MODEL_CACHE_PATH: model_cache},
)
class GraniteSpeech:
    """Warm, GPU-backed Granite Speech transcription service."""

    @modal.enter()  # pyright: ignore[reportUnknownMemberType]
    def load_model(self) -> None:
        import torch  # pyright: ignore[reportMissingImports]
        from transformers import AutoModelForSpeechSeq2Seq, AutoProcessor  # pyright: ignore[reportMissingImports, reportUnknownVariableType]

        self._torch = cast(_Torch, torch)
        if not self._torch.cuda.is_available():
            raise RuntimeError("Granite Speech requires a CUDA GPU")

        self._device = "cuda"
        self._processor = cast(
            _Processor,
            AutoProcessor.from_pretrained(  # pyright: ignore[reportUnknownMemberType]
                MODEL_ID,
                revision=MODEL_REVISION,
                cache_dir=MODEL_CACHE_PATH,
            ),
        )
        self._model = cast(
            _Model,
            AutoModelForSpeechSeq2Seq.from_pretrained(  # pyright: ignore[reportUnknownMemberType]
                MODEL_ID,
                revision=MODEL_REVISION,
                cache_dir=MODEL_CACHE_PATH,
                torch_dtype=self._torch.bfloat16,
            ),
        ).to(self._device)
        self._model.eval()
        model_cache.commit()

    @modal.method()  # pyright: ignore[reportUnknownMemberType]
    def transcribe(self, audio_url: str) -> str:
        """Transcribe audio fetched from an absolute HTTPS URL."""
        with TemporaryDirectory() as temporary_directory:
            raw_audio = Path(temporary_directory, "source-audio")
            _download_audio(audio_url, raw_audio)
            return self._transcribe_file(raw_audio)

    @modal.method()  # pyright: ignore[reportUnknownMemberType]
    def transcribe_bytes(self, audio: bytes) -> str:
        """Transcribe in-memory audio, used by local development tooling."""
        if len(audio) > MAX_AUDIO_BYTES:
            raise ValueError("audio file exceeds the 512 MiB limit")
        with TemporaryDirectory() as temporary_directory:
            raw_audio = Path(temporary_directory, "source-audio")
            raw_audio.write_bytes(audio)
            return self._transcribe_file(raw_audio)

    def _transcribe_file(self, source_audio: Path) -> str:
        with TemporaryDirectory() as temporary_directory:
            normalized_audio = Path(temporary_directory, "audio.wav")
            _normalize_audio(source_audio, normalized_audio)
            waveform = _read_audio(normalized_audio)

        return self._generate_transcript(waveform)

    def _generate_transcript(self, waveform: object) -> str:
        tokenizer = self._processor.tokenizer
        prompt = tokenizer.apply_chat_template(
            [{"role": "user", "content": TRANSCRIPTION_PROMPT}],
            tokenize=False,
            add_generation_prompt=True,
        )
        processed = cast(
            _Movable,
            self._processor(
                prompt,
                waveform,
                device=self._device,
                return_tensors="pt",
            ),
        ).to(self._device)
        model_inputs = cast(Mapping[str, object], processed)
        input_ids = cast(_Tensor, model_inputs["input_ids"])

        with self._torch.inference_mode():
            generated = self._model.generate(
                **model_inputs,
                max_new_tokens=MAX_NEW_TOKENS,
                do_sample=False,
                num_beams=1,
            )

        input_token_count = input_ids.shape[-1]
        new_tokens = generated[0, input_token_count:].unsqueeze(0)
        decoded = tokenizer.batch_decode(
            new_tokens,
            add_special_tokens=False,
            skip_special_tokens=True,
        )
        return decoded[0].strip()


def _download_audio(audio_url: str, destination: Path) -> None:
    download_audio(audio_url, destination, max_bytes=MAX_AUDIO_BYTES)


def _normalize_audio(source: Path, destination: Path) -> None:
    normalize_audio(source, destination, max_seconds=MAX_AUDIO_SECONDS)


def _read_audio(audio_path: Path) -> object:
    validate_wav(audio_path, max_seconds=MAX_AUDIO_SECONDS)

    from soundfile import read  # pyright: ignore[reportMissingImports, reportUnknownVariableType]

    raw_waveform, sample_rate = cast(Callable[..., tuple[object, int]], read)(
        audio_path,
        dtype="float32",
        always_2d=True,
    )
    waveform = cast(_AudioArray, raw_waveform)
    if sample_rate != 16_000 or len(waveform.shape) != 2:
        raise RuntimeError("failed to normalize audio to mono 16 kHz")
    if waveform.shape[0] == 0 or waveform.shape[1] != 1:
        raise ValueError("audio must contain at least one sample")
    return waveform.T
