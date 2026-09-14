"""Shared transcription domain contracts."""

from dataclasses import dataclass
from enum import StrEnum

from socialguard_models.contracts import AudioTask, FailedOutcome


class ModelType(StrEnum):
    """Transcription models exposed by the service."""

    GRANITE = "granite"


@dataclass(frozen=True, slots=True)
class TranscriptionTask(AudioTask):
    """A transcription request and its workflow correlation data."""

    model: ModelType


@dataclass(frozen=True, slots=True)
class CompletedOutcome:
    """The successful outcome of a transcription worker."""

    text: str


type TranscriptionOutcome = CompletedOutcome | FailedOutcome
