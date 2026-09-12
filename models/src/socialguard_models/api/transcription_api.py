"""Transcription HTTP route and request/response contracts."""

from typing import Literal

from fastapi import APIRouter
from pydantic import BaseModel, ConfigDict

from socialguard_models.transcription import (
    CompletedOutcome,
    ModelType,
    TranscriptionOutcome,
    TranscriptionTask,
    run_transcription,
)

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


class _BaseTranscriptionResponse(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: ModelType
    idempotency_key: str
    pipeline_task_id: str


class CompletedTranscriptionResponse(_BaseTranscriptionResponse):
    status: Literal["completed"] = "completed"
    text: str


class FailedTranscriptionResponse(_BaseTranscriptionResponse):
    status: Literal["failed"] = "failed"
    error_code: Literal["transcription_failed"] = "transcription_failed"


type TranscriptionResponse = (
    CompletedTranscriptionResponse | FailedTranscriptionResponse
)


@router.post("/")
def transcribe(request: TranscriptionRequest) -> TranscriptionResponse:
    task = TranscriptionTask(
        model=request.model,
        audio_uri=request.audio_uri,
        idempotency_key=request.idempotency_key,
        pipeline_task_id=request.pipeline_task_id,
        task_token=request.task_token,
    )
    outcome = run_transcription(task)
    return response_for_outcome(task, outcome)


def response_for_outcome(
    task: TranscriptionTask,
    outcome: TranscriptionOutcome,
) -> TranscriptionResponse:
    if isinstance(outcome, CompletedOutcome):
        return CompletedTranscriptionResponse(
            model=task.model,
            text=outcome.text,
            idempotency_key=task.idempotency_key,
            pipeline_task_id=task.pipeline_task_id,
        )

    return FailedTranscriptionResponse(
        model=task.model,
        idempotency_key=task.idempotency_key,
        pipeline_task_id=task.pipeline_task_id,
    )
