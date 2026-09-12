from concurrent.futures import ThreadPoolExecutor
from threading import Barrier

import pytest
from fastapi.testclient import TestClient

import socialguard_models.cpu_worker as cpu_worker
import socialguard_models.api.transcription_api as transcription_api
from socialguard_models.api.transcription_api import TranscriptionRequest
from socialguard_models.transcription import (
    CompletedOutcome,
    FailedOutcome,
    TranscriptionOutcome,
    TranscriptionTask,
)


def test_concurrent_requests_keep_their_own_tasks_and_responses(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    barrier = Barrier(2)

    def run_transcription(task: TranscriptionTask) -> TranscriptionOutcome:
        assert task.audio_uri == f"s3://example/{task.pipeline_task_id}.wav"
        assert task.task_token == f"token-{task.pipeline_task_id}"
        assert task.idempotency_key == f"request-{task.pipeline_task_id}"
        barrier.wait(timeout=5)
        if task.pipeline_task_id == "failed":
            return FailedOutcome(error_type="RuntimeError")
        return CompletedOutcome(text="A complete transcript.")

    monkeypatch.setattr(transcription_api, "run_transcription", run_transcription)
    requests = [
        TranscriptionRequest(
            model="granite",
            audio_uri=f"s3://example/{task_id}.wav",
            idempotency_key=f"request-{task_id}",
            pipeline_task_id=task_id,
            task_token=f"token-{task_id}",
        )
        for task_id in ("completed", "failed")
    ]

    with TestClient(cpu_worker.serve_api.local()) as client:
        def post(request: TranscriptionRequest):
            return client.post("/", json=request.model_dump(mode="json"))

        with ThreadPoolExecutor(max_workers=2) as executor:
            responses = list(executor.map(post, requests))

    assert all(response.status_code == 200 for response in responses)
    assert [response.json() for response in responses] == [
        {
            "status": "completed",
            "model": "granite",
            "text": "A complete transcript.",
            "idempotency_key": "request-completed",
            "pipeline_task_id": "completed",
        },
        {
            "status": "failed",
            "model": "granite",
            "error_code": "transcription_failed",
            "idempotency_key": "request-failed",
            "pipeline_task_id": "failed",
        },
    ]
