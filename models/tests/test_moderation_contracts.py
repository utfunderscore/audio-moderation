import json

import pytest
from pydantic import ValidationError

from socialguard_models.api.moderation_api import ModerationRequest
from socialguard_models.api.transcription_api import TranscriptionRequest
from socialguard_models.contracts import FailedOutcome
from socialguard_models.moderation.callbacks import completion_outcome
from socialguard_models.moderation.contracts import (
    CompletedOutcome,
    ModerationScores,
    ModerationTask,
)


def test_moderation_accepts_transcription_inputs_plus_transcription() -> None:
    transcription_request = TranscriptionRequest(
        model="granite",
        audio_uri="s3://example/audio.wav",
        idempotency_key="request-123",
        pipeline_task_id="task-123",
        task_token="token-123",
    )
    inputs = transcription_request.model_dump(mode="json")
    inputs["model"] = "future-moderation-model"
    inputs["transcription"] = "Words spoken in the audio."
    request = ModerationRequest.model_validate(inputs)
    task = ModerationTask(**request.model_dump())

    assert request.model_dump() == inputs
    assert task.audio_uri == transcription_request.audio_uri
    assert task.idempotency_key == transcription_request.idempotency_key
    assert task.pipeline_task_id == transcription_request.pipeline_task_id
    assert task.task_token == transcription_request.task_token
    assert task.transcription == inputs["transcription"]


@pytest.mark.parametrize("transcription", [None, 123, {}, []])
def test_moderation_transcription_must_be_a_string(transcription: object) -> None:
    with pytest.raises(ValidationError):
        ModerationRequest.model_validate({
            "model": "future-moderation-model",
            "audio_uri": "s3://example/audio.wav",
            "idempotency_key": "request-123",
            "pipeline_task_id": "task-123",
            "task_token": "token-123",
            "transcription": transcription,
        })


def test_moderation_success_callback_is_a_json_score_map() -> None:
    scores: ModerationScores = {
        "sexual": 0.01,
        "hate_or_discrimination": 0.02,
        "harassment_or_abuse": 0.03,
        "violence_or_threats": 0.04,
        "asking_for_pii": 0.05,
    }
    payload = {
        "taskToken": "token-123",
        "outcome": completion_outcome(
            job_id="task-123",
            moderation_task_id="moderation-456",
            outcome=CompletedOutcome(scores=scores),
        ),
    }
    decoded = json.loads(json.dumps(payload))
    assert decoded == {
        "taskToken": "token-123",
        "outcome": {
            "type": "success",
            "moderationResult": {
                "jobId": "task-123",
                "moderationTaskId": "moderation-456",
                "scores": scores,
            },
        },
    }
    assert all(isinstance(value, float) for value in decoded["outcome"]["moderationResult"]["scores"].values())


def test_moderation_failure_callback_uses_shared_failure_envelope() -> None:
    assert completion_outcome(
        job_id="task-123",
        moderation_task_id="moderation-456",
        outcome=FailedOutcome(cause="TimeoutError"),
    ) == {
        "type": "failure",
        "error": "ModerationFailed",
        "cause": "TimeoutError",
    }
