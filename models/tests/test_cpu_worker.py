from fastapi.testclient import TestClient
import pytest
from unittest.mock import AsyncMock, Mock

import socialguard_models.idempotency as idempotency
import socialguard_models.api.submission as submission
import socialguard_models.cpu_worker as cpu_worker
from socialguard_models.moderation.contracts import ModerationTask
from socialguard_models.moderation.job import ModerationJob
from socialguard_models.moderation.models.registry import MODEL_SUBMITTERS


class TaskIds:
    def __init__(self) -> None:
        self.values: dict[str, object] = {}

    def put(self, key: str, value: object, *, skip_if_exists: bool = False) -> bool:
        if skip_if_exists and key in self.values:
            return False
        self.values[key] = value
        return True

    def get(self, key: str) -> object | None:
        return self.values.get(key)

    def pop(self, key: str, default: object) -> str | object:
        return self.values.pop(key, default)


@pytest.mark.parametrize("path", ["/", "/transcription/"])
def test_http_request_returns_before_transcription_runs(
    monkeypatch: pytest.MonkeyPatch,
    path: str,
) -> None:
    task_ids = TaskIds()
    scheduler = Mock()
    monkeypatch.setattr(idempotency, "task_ids", task_ids)
    monkeypatch.setattr(submission, "process_model", scheduler)

    with TestClient(cpu_worker.serve_api.local()) as client:
        response = client.post(
            path,
            json={
                "model": "granite",
                "audio_uri": "s3://example/audio.wav",
                "idempotency_key": "request-123",
                "pipeline_task_id": "task-123",
                "task_token": "token-123",
            },
        )

    assert response.status_code == 202
    assert response.json()["status"] == "queued"
    assert response.json()["task_id"].startswith("transcription_")
    scheduler.spawn.assert_called_once()


def test_model_routes_have_separate_input_contracts(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    scheduler = Mock()
    monkeypatch.setattr(submission, "process_model", scheduler)
    request = {
        "model": "future-moderation-model",
        "audio_uri": "s3://example/audio.wav",
        "idempotency_key": "request-123",
        "pipeline_task_id": "task-123",
        "task_token": "token-123",
    }
    with TestClient(cpu_worker.serve_api.local()) as client:
        assert client.post("/moderation/", json=request).status_code == 422
        request["transcription"] = "Text to moderate."
        assert client.post("/moderation/", json=request).status_code == 422
        for model in tuple(MODEL_SUBMITTERS):
            monkeypatch.delitem(MODEL_SUBMITTERS, model)
        assert client.post("/moderation/", json=request).status_code == 501
        request["model"] = "granite"
        assert client.post("/transcription/", json=request).status_code == 422
        paths = client.get("/openapi.json").json()["paths"]
        assert "/transcription/" in paths
        assert "/moderation/" in paths
        assert "/" not in paths
    scheduler.spawn.assert_not_called()


def test_scheduling_keys_are_isolated_by_model_family(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(idempotency, "task_ids", TaskIds())
    transcription_dispatch = Mock()
    moderation_dispatch = Mock()
    transcription_id = idempotency.schedule_idempotently("same-key", transcription_dispatch)
    moderation_id = idempotency.schedule_idempotently(
        "same-key", moderation_dispatch, family="moderation"
    )
    assert transcription_id.startswith("transcription_")
    assert moderation_id.startswith("moderation_")
    assert idempotency.schedule_idempotently("same-key", transcription_dispatch) == transcription_id
    assert idempotency.schedule_idempotently(
        "same-key", moderation_dispatch, family="moderation"
    ) == moderation_id
    transcription_dispatch.assert_called_once_with(transcription_id)
    moderation_dispatch.assert_called_once_with(moderation_id)


def test_moderation_route_queues_real_job_and_replays_accepted_id(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    scheduler = Mock()
    monkeypatch.setattr(idempotency, "task_ids", TaskIds())
    monkeypatch.setattr(submission, "process_model", scheduler)
    request = {
        "model": "roblox-voice-safety-v3",
        "audio_uri": "s3://example/audio.wav",
        "transcription": "Words spoken in the audio.",
        "idempotency_key": "request-123",
        "pipeline_task_id": "task-123",
        "task_token": "token-123",
    }

    with TestClient(cpu_worker.serve_api.local()) as client:
        first = client.post("/moderation/", json=request)
        second = client.post("/moderation/", json=request)

    assert first.status_code == second.status_code == 202
    assert first.json() == second.json()
    assert first.json()["status"] == "queued"
    assert first.json()["task_id"].startswith("moderation_")
    scheduler.spawn.assert_called_once_with(
        ModerationJob(ModerationTask(**request)), first.json()["task_id"]
    )


def test_moderation_route_rejects_unknown_model_before_scheduling(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    scheduler = Mock()
    monkeypatch.setitem(MODEL_SUBMITTERS, "example", AsyncMock())
    monkeypatch.setattr(submission, "process_model", scheduler)

    with TestClient(cpu_worker.serve_api.local()) as client:
        response = client.post("/moderation/", json={
            "model": "unknown",
            "audio_uri": "s3://example/audio.wav",
            "transcription": "Words spoken in the audio.",
            "idempotency_key": "request-123",
            "pipeline_task_id": "task-123",
            "task_token": "token-123",
        })

    assert response.status_code == 422
    scheduler.spawn.assert_not_called()
