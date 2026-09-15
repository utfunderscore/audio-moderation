"""Moderation submission through the shared model execution pipeline."""

from fastapi import APIRouter, HTTPException, status

from socialguard_models.api.contracts import AudioRequest, QueuedResponse
from socialguard_models.api.submission import submit_model
from socialguard_models.moderation.contracts import ModerationTask
from socialguard_models.moderation.job import ModerationJob
from socialguard_models.moderation.models.registry import MODEL_SUBMITTERS

router = APIRouter()


class ModerationRequest(AudioRequest):
    """An audio file reference and its transcription for moderation."""

    model: str
    transcription: str


@router.post("/", status_code=status.HTTP_202_ACCEPTED)
def moderate(request: ModerationRequest) -> QueuedResponse:
    """Validate model availability, then accept work without waiting for inference."""
    if not MODEL_SUBMITTERS:
        raise HTTPException(
            status_code=status.HTTP_501_NOT_IMPLEMENTED,
            detail="moderation models are not implemented yet",
        )
    if request.model not in MODEL_SUBMITTERS:
        raise HTTPException(
            status_code=status.HTTP_422_UNPROCESSABLE_ENTITY,
            detail="unsupported moderation model",
        )
    task = ModerationTask(
        model=request.model,
        audio_uri=request.audio_uri,
        transcription=request.transcription,
        idempotency_key=request.idempotency_key,
        pipeline_task_id=request.pipeline_task_id,
        task_token=request.task_token,
    )
    return submit_model(ModerationJob(task))
