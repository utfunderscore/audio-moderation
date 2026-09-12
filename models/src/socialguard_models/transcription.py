"""Shared transcription contracts, model routing, and orchestration."""

import logging
from dataclasses import dataclass
from enum import StrEnum
from time import monotonic
from typing import Protocol

from socialguard_models.aws import assume_modal_oidc_role, create_presigned_download_url
from socialguard_models.granite import GraniteSpeech

logger = logging.getLogger(__name__)
WORKER_TIMEOUT_SECONDS = 660


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

    error_type: str


type TranscriptionOutcome = CompletedOutcome | FailedOutcome


class SubmittedTranscription(Protocol):
    """A submitted worker call whose result can be awaited or cancelled."""

    def get(self, timeout: float | None = None) -> str: ...

    def cancel(self) -> None: ...


def run_transcription(task: TranscriptionTask) -> TranscriptionOutcome:
    """Resolve S3 audio, run the selected GPU worker, and log its outcome."""
    deadline = monotonic() + WORKER_TIMEOUT_SECONDS
    worker_call: SubmittedTranscription | None = None
    try:
        session = assume_modal_oidc_role()
        audio_url = create_presigned_download_url(session, task.audio_uri)
        match task.model:
            case ModelType.GRANITE:
                worker_call = GraniteSpeech().transcribe.spawn(audio_url)  # pyright: ignore[reportAttributeAccessIssue]
            case _:  # pyright: ignore[reportUnnecessaryComparison]
                raise ValueError(f"unsupported transcription model: {task.model}")

        remaining_seconds = deadline - monotonic()
        if remaining_seconds <= 0:
            raise TimeoutError("transcription worker deadline exceeded")
        text = worker_call.get(timeout=remaining_seconds)
    except Exception as error:
        if worker_call is not None:
            try:
                worker_call.cancel()
            except Exception:
                pass
        logger.warning(
            "transcription worker failed: pipeline_task_id=%s model=%s error_type=%s",
            task.pipeline_task_id,
            task.model,
            type(error).__name__,
        )
        return FailedOutcome(error_type=type(error).__name__)

    logger.info(
        "transcription worker completed: pipeline_task_id=%s model=%s",
        task.pipeline_task_id,
        task.model,
    )
    return CompletedOutcome(text=text)
