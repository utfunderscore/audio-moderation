"""Model-independent task metadata and failure contracts."""

from dataclasses import dataclass


@dataclass(frozen=True, slots=True, kw_only=True)
class AudioTask:
    """An S3 audio input and workflow correlation data."""

    audio_uri: str
    idempotency_key: str
    pipeline_task_id: str
    task_token: str


@dataclass(frozen=True, slots=True)
class FailedOutcome:
    """A handled model worker failure."""

    cause: str
