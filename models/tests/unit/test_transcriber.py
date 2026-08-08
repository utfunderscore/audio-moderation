"""Tests for the structural transcriber interface."""

from pathlib import Path

from socialguard_models.core import (
    AudioInput,
    ModelCapabilities,
    ModelInfo,
    Transcriber,
    Transcript,
    TranscriptionOptions,
)


class FakeTranscriber:
    """Small typed backend used to verify structural compatibility."""

    @property
    def info(self) -> ModelInfo:
        """Return fixed model metadata."""
        return ModelInfo(
            model_id="fake-asr",
            engine="fake",
            revision="test-revision",
            capabilities=ModelCapabilities(),
        )

    def transcribe(
        self,
        audio: AudioInput,
        options: TranscriptionOptions,
    ) -> Transcript:
        """Return a deterministic transcript for any input."""
        del audio, options
        return Transcript(text="test transcript")


def _run_transcriber(transcriber: Transcriber) -> Transcript:
    """Accept only implementations satisfying the public protocol."""
    return transcriber.transcribe(
        AudioInput(path=Path("fixture.wav")),
        TranscriptionOptions(),
    )


def test_backend_satisfies_transcriber_protocol() -> None:
    """A structural implementation can be consumed through the protocol."""
    transcript = _run_transcriber(FakeTranscriber())

    assert transcript == Transcript(text="test transcript")
