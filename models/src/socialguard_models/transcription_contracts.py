"""Shared transcription domain contracts."""

from dataclasses import dataclass
from enum import StrEnum


class ModelType(StrEnum):
    """Transcription models exposed by the service."""

    GRANITE = "granite"


@dataclass(frozen=True, slots=True)
class TranscriptionTask:
    """A transcription request and its workflow correlation data."""

    model: ModelType
    audio_uri: str
    idempotency_key: str
    pipeline_task_id: str
    task_token: str


@dataclass(frozen=True, slots=True)
class CompletedOutcome:
    """The successful outcome of a transcription worker."""

    text: str


@dataclass(frozen=True, slots=True)
class FailedOutcome:
    """The handled failure of a transcription worker."""

    cause: str


type TranscriptionOutcome = CompletedOutcome | FailedOutcome
