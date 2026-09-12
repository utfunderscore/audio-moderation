"""Shared transcription contracts, model routing, and orchestration."""

import asyncio
import logging
from time import monotonic
from typing import Awaitable, Protocol, cast

import modal

from socialguard_models.aws import assume_modal_oidc_role, create_presigned_download_url
from socialguard_models.callbacks import (
    CALLBACK_MAX_DURATION_SECONDS,
    get_callback_uri,
    post_completion_callback,
)
from socialguard_models.granite import GraniteSpeech
from socialguard_models.modal_app import app, cpu_image
from socialguard_models.transcription_contracts import (
    CompletedOutcome,
    FailedOutcome,
    ModelType,
    TranscriptionTask,
    TranscriptionOutcome,
)

logger = logging.getLogger(__name__)
WORKER_TIMEOUT_SECONDS = 660
MAX_CONCURRENT_TRANSCRIPTIONS = 32
PROCESS_TIMEOUT_SECONDS = WORKER_TIMEOUT_SECONDS + CALLBACK_MAX_DURATION_SECONDS


class _AsyncResult(Protocol):
    """The asynchronous variant of a Modal remote method."""

    def aio(self, timeout: float | None = None) -> Awaitable[str]: ...


class _AsyncCancellation(Protocol):
    """The asynchronous variant of Modal's cancellation method."""

    def aio(self) -> Awaitable[None]: ...


class SubmittedTranscription(Protocol):
    """A submitted worker call whose result can be awaited or cancelled."""

    get: _AsyncResult

    cancel: _AsyncCancellation


async def run_transcription(task: TranscriptionTask) -> TranscriptionOutcome:
    """Resolve S3 audio, run the selected GPU worker, and log its outcome."""
    deadline = monotonic() + WORKER_TIMEOUT_SECONDS
    worker_call: SubmittedTranscription | None = None
    try:
        session = await asyncio.to_thread(assume_modal_oidc_role)
        audio_url = await asyncio.to_thread(
            create_presigned_download_url,
            session,
            task.audio_uri,
        )
        match task.model:
            case ModelType.GRANITE:
                worker_call = cast(
                    SubmittedTranscription,
                    await GraniteSpeech().transcribe.spawn.aio(audio_url),  # pyright: ignore[reportAttributeAccessIssue, reportUnknownMemberType]
                )
            case _:  # pyright: ignore[reportUnnecessaryComparison]
                raise ValueError(f"unsupported transcription model: {task.model}")

        remaining_seconds = deadline - monotonic()
        if remaining_seconds <= 0:
            raise TimeoutError("transcription worker deadline exceeded")
        assert worker_call is not None
        text = await worker_call.get.aio(timeout=remaining_seconds)
    except Exception as error:
        if worker_call is not None:
            try:
                await worker_call.cancel.aio()
            except Exception:
                pass
        logger.warning(
            "transcription worker failed: pipeline_task_id=%s model=%s error_type=%s",
            task.pipeline_task_id,
            task.model,
            type(error).__name__,
        )
        return FailedOutcome(
            cause=type(error).__name__,
        )

    logger.info(
        "transcription worker completed: pipeline_task_id=%s model=%s",
        task.pipeline_task_id,
        task.model,
    )
    return CompletedOutcome(text=text)


@app.function(  # pyright: ignore[reportUnknownMemberType]
    image=cpu_image,
    timeout=PROCESS_TIMEOUT_SECONDS,
)
@modal.concurrent(max_inputs=MAX_CONCURRENT_TRANSCRIPTIONS)  # pyright: ignore[reportUnknownMemberType]
async def process_transcription(task: TranscriptionTask, task_id: str) -> None:
    """Run accepted work outside the HTTP request lifecycle."""
    callback_uri = get_callback_uri()
    outcome = await run_transcription(task)
    await asyncio.to_thread(
        post_completion_callback,
        callback_uri=callback_uri,
        task_token=task.task_token,
        job_id=task.pipeline_task_id,
        asr_task_id=task_id,
        outcome=outcome,
    )
    logger.info(
        "transcription task reached a terminal outcome: task_id=%s outcome_type=%s",
        task_id,
        type(outcome).__name__,
    )
