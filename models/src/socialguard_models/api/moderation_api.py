"""Separate moderation API boundary, ready for model implementation."""

from fastapi import APIRouter, HTTPException, status

from socialguard_models.api.contracts import AudioRequest, QueuedResponse

router = APIRouter()


class ModerationRequest(AudioRequest):
    """An audio file reference and its transcription for moderation."""

    model: str
    transcription: str


@router.post("/", status_code=status.HTTP_202_ACCEPTED)
def moderate(request: ModerationRequest) -> QueuedResponse:
    """Reserve the contract without accepting work until a model is implemented."""
    raise HTTPException(
        status_code=status.HTTP_501_NOT_IMPLEMENTED,
        detail="moderation models are not implemented yet",
    )
