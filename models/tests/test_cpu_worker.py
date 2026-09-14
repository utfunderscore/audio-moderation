from fastapi.testclient import TestClient
import pytest
from unittest.mock import Mock

import socialguard_models.idempotency as idempotency
import socialguard_models.api.submission as submission
import socialguard_models.cpu_worker as cpu_worker


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
