"""Transcription submission HTTP route and request/response contracts."""

from fastapi import APIRouter, status
from socialguard_models.api.contracts import AudioRequest, QueuedResponse
from socialguard_models.api.submission import submit_model
from socialguard_models.transcription.job import TranscriptionJob
from socialguard_models.transcription.contracts import ModelType, TranscriptionTask

router = APIRouter()


class TranscriptionRequest(AudioRequest):
    model: ModelType


QueuedTranscriptionResponse = QueuedResponse
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

    return submit_model(TranscriptionJob(task))
