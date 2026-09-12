"""Transcription submission HTTP route and request/response contracts."""

from typing import Literal

from fastapi import APIRouter, HTTPException, status
from pydantic import BaseModel, ConfigDict


from socialguard_models.idempotency import SchedulingUnavailableError, schedule_idempotently
from socialguard_models.transcription import process_transcription
from socialguard_models.transcription_contracts import ModelType, TranscriptionTask

router = APIRouter()


class TranscriptionRequest(BaseModel):
    model_config = ConfigDict(
        extra="forbid",
        json_schema_extra={
            "$schema": "https://json-schema.org/draft/2020-12/schema"
        },
    )

    model: ModelType
    audio_uri: str
    idempotency_key: str
    pipeline_task_id: str
    task_token: str


class QueuedTranscriptionResponse(BaseModel):
    """An accepted transcription task that will complete asynchronously."""

    model_config = ConfigDict(extra="forbid")

    status: Literal["queued"] = "queued"
    task_id: str


@router.post("/", status_code=status.HTTP_202_ACCEPTED)
def transcribe(request: TranscriptionRequest) -> QueuedTranscriptionResponse:
    """Atomically accept one task per idempotency key without awaiting execution."""
    task = TranscriptionTask(
        model=request.model,
        audio_uri=request.audio_uri,
        idempotency_key=request.idempotency_key,
        pipeline_task_id=request.pipeline_task_id,
        task_token=request.task_token,
    )

    def dispatch(task_id: str) -> None:
        process_transcription.spawn(task, task_id)  # pyright: ignore[reportFunctionMemberAccess]

    try:
        task_id = schedule_idempotently(task.idempotency_key, dispatch)
    except SchedulingUnavailableError as error:
        raise HTTPException(
            status_code=status.HTTP_503_SERVICE_UNAVAILABLE,
            detail="unable to schedule transcription",
        ) from error
    return QueuedTranscriptionResponse(task_id=task_id)
