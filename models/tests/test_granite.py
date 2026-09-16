import wave
from contextlib import AbstractContextManager, nullcontext
from pathlib import Path
from typing import Self

import pytest

import socialguard_models.transcription.models.granite as granite


@pytest.mark.parametrize(
    "audio_url",
    [
        "http://example.com/audio.wav",
        "s3://example/audio.wav",
        "not-a-url",
    ],
)
def test_download_audio_requires_https(audio_url: str, tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="HTTPS"):
        granite._download_audio(audio_url, tmp_path / "audio")


def test_transcribe_bytes_normalizes_in_memory_audio(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    worker = granite.GraniteSpeech._get_user_cls()()
    captured: dict[str, bytes] = {}

    def fake_normalize_audio(source: Path, destination: Path) -> None:
        captured["source"] = source.read_bytes()
        destination.write_bytes(b"normalized")

    def fake_read_audio(audio_path: Path) -> object:
        captured["read"] = audio_path.read_bytes()
        return "waveform"

    monkeypatch.setattr(granite, "_normalize_audio", fake_normalize_audio)
    monkeypatch.setattr(granite, "_read_audio", fake_read_audio)
    monkeypatch.setattr(worker, "_generate_transcript", lambda waveform: "transcript")

    assert worker.transcribe_bytes(b"raw-audio") == "transcript"  # pyright: ignore[reportAttributeAccessIssue, reportUnknownMemberType]
    assert captured == {"source": b"raw-audio", "read": b"normalized"}


def test_transcribe_bytes_rejects_audio_over_size_limit(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    worker = granite.GraniteSpeech._get_user_cls()()
    monkeypatch.setattr(granite, "MAX_AUDIO_BYTES", 4)
    monkeypatch.setattr(worker, "_transcribe_file", lambda source: "transcript")

    assert worker.transcribe_bytes(b"1234") == "transcript"  # pyright: ignore[reportAttributeAccessIssue, reportUnknownMemberType]
    with pytest.raises(ValueError, match="audio file exceeds the 512 MiB limit"):
        worker.transcribe_bytes(b"12345")  # pyright: ignore[reportAttributeAccessIssue, reportUnknownMemberType]


def test_read_audio_rejects_audio_over_duration_limit(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    audio_path = tmp_path / "audio.wav"
    with wave.open(str(audio_path), "wb") as wav_file:
        wav_file.setnchannels(1)
        wav_file.setsampwidth(2)
        wav_file.setframerate(16_000)
        wav_file.writeframes(b"\0\0" * 32_000)
    monkeypatch.setattr(granite, "MAX_AUDIO_SECONDS", 1)

    with pytest.raises(ValueError, match="1 seconds"):
        granite._read_audio(audio_path)


class FakeTensor:
    def __init__(self, shape: tuple[int, ...]) -> None:
        self.shape = shape
        self.last_key: object | None = None
        self.unsqueezeed_dimension: int | None = None

    def __getitem__(self, key: object) -> Self:
        self.last_key = key
        return self

    def unsqueeze(self, dim: int) -> Self:
        self.unsqueezeed_dimension = dim
        return self


class FakeBatch(dict[str, object]):
    moved_to: str | None = None

    def to(self, device: str) -> Self:
        self.moved_to = device
        return self


class FakeTokenizer:
    decoded_tokens: FakeTensor | None = None

    def apply_chat_template(
        self,
        conversation: list[dict[str, str]],
        *,
        tokenize: bool,
        add_generation_prompt: bool,
    ) -> str:
        assert conversation == [
            {"role": "user", "content": granite.TRANSCRIPTION_PROMPT}
        ]
        assert tokenize is False
        assert add_generation_prompt is True
        return "formatted prompt"

    def batch_decode(
        self,
        sequences: granite._Tensor,
        *,
        add_special_tokens: bool,
        skip_special_tokens: bool,
    ) -> list[str]:
        assert isinstance(sequences, FakeTensor)
        self.decoded_tokens = sequences
        assert add_special_tokens is False
        assert skip_special_tokens is True
        return ["  A complete transcript.  "]


class FakeProcessor:
    def __init__(self, tokenizer: FakeTokenizer, inputs: FakeBatch) -> None:
        self.tokenizer: granite._Tokenizer = tokenizer
        self.inputs = inputs

    def __call__(
        self,
        text: str,
        audio: object,
        *,
        device: str,
        return_tensors: str,
    ) -> FakeBatch:
        assert text == "formatted prompt"
        assert audio == "waveform"
        assert device == "cuda"
        assert return_tensors == "pt"
        return self.inputs


class FakeModel:
    def __init__(self, generated: FakeTensor) -> None:
        self.generated = generated
        self.generation_arguments: dict[str, object] = {}

    def to(self, device: str) -> Self:
        return self

    def eval(self) -> None:
        pass

    def generate(self, **kwargs: object) -> FakeTensor:
        self.generation_arguments = kwargs
        return self.generated


class FakeCuda:
    def is_available(self) -> bool:
        return True


class FakeTorch:
    bfloat16 = object()
    cuda: granite._Cuda = FakeCuda()

    def inference_mode(self) -> AbstractContextManager[None]:
        return nullcontext()


def test_generate_transcript_uses_deterministic_text_only_generation() -> None:
    input_ids = FakeTensor((1, 4))
    inputs = FakeBatch(input_ids=input_ids, input_features="features")
    generated = FakeTensor((1, 7))
    tokenizer = FakeTokenizer()
    model = FakeModel(generated)

    # Use the undecorated class to exercise inference without a Modal container.
    worker = granite.GraniteSpeech._get_user_cls()()
    worker._processor = FakeProcessor(tokenizer, inputs)
    worker._model = model
    worker._torch = FakeTorch()
    worker._device = "cuda"

    result = worker._generate_transcript("waveform")

    assert result == "A complete transcript."
    assert inputs.moved_to == "cuda"
    assert model.generation_arguments == {
        "input_ids": input_ids,
        "input_features": "features",
        "max_new_tokens": granite.MAX_NEW_TOKENS,
        "do_sample": False,
        "num_beams": 1,
    }
    assert generated.last_key == (0, slice(4, None))
    assert generated.unsqueezeed_dimension == 0
    assert tokenizer.decoded_tokens is generated
