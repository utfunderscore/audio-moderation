import pytest
from pydantic import ValidationError

from socialguard_models.api.transcription_api import (
    CompletedTranscriptionResponse,
    FailedTranscriptionResponse,
    TranscriptionRequest,
    response_for_outcome,
)
from socialguard_models.transcription import (
    CompletedOutcome,
    FailedOutcome,
    ModelType,
    TranscriptionTask,
)


def make_task() -> TranscriptionTask:
    return TranscriptionTask(
        model=ModelType.GRANITE,
        audio_uri="s3://example/audio.wav",
        idempotency_key="request-123",
        pipeline_task_id="task-123",
        task_token="token-123",
    )


def test_completed_response_contains_text_without_segments_or_speakers() -> None:
    response = response_for_outcome(
        make_task(),
        CompletedOutcome(text="A complete transcript."),
    )

    assert isinstance(response, CompletedTranscriptionResponse)
    assert response.model_dump(mode="json") == {
        "status": "completed",
        "model": "granite",
        "text": "A complete transcript.",
        "idempotency_key": "request-123",
        "pipeline_task_id": "task-123",
    }


def test_failed_response_contains_worker_error_without_text() -> None:
    response = response_for_outcome(
        make_task(),
        FailedOutcome(error_type="RuntimeError"),
    )

    assert isinstance(response, FailedTranscriptionResponse)
    assert response.model_dump(mode="json") == {
        "status": "failed",
        "model": "granite",
        "error_code": "transcription_failed",
        "idempotency_key": "request-123",
        "pipeline_task_id": "task-123",
    }


def test_request_rejects_unknown_fields() -> None:
    with pytest.raises(ValidationError):
        TranscriptionRequest.model_validate(
            {
                "model": "granite",
                "audio_uri": "s3://example/audio.wav",
                "idempotency_key": "request-123",
                "pipeline_task_id": "task-123",
                "task_token": "token-123",
                "timestamps": True,
            }
        )
