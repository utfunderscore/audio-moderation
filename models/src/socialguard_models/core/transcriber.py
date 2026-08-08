"""Structural interface implemented by every ASR backend."""

from typing import Protocol

from socialguard_models.core.models import (
    AudioInput,
    ModelInfo,
    Transcript,
    TranscriptionOptions,
)


class Transcriber(Protocol):
    """Model-independent interface for synchronous ASR inference."""

    @property
    def info(self) -> ModelInfo:
        """Return immutable identity and capability information."""
        ...

    def transcribe(
        self,
        audio: AudioInput,
        options: TranscriptionOptions,
    ) -> Transcript:
        """Transcribe one prepared audio input."""
        ...
