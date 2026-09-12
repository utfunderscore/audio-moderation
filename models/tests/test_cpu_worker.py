from fastapi.testclient import TestClient
import pytest
from unittest.mock import Mock

import socialguard_models.idempotency as idempotency
import socialguard_models.api.transcription_api as transcription_api
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


def test_http_request_returns_before_transcription_runs(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    task_ids = TaskIds()
    scheduler = Mock()
    monkeypatch.setattr(idempotency, "task_ids", task_ids)
    monkeypatch.setattr(transcription_api, "process_transcription", scheduler)

    with TestClient(cpu_worker.serve_api.local()) as client:
        response = client.post(
            "/",
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
