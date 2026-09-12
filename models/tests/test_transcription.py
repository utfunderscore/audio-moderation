import logging
from unittest.mock import Mock

import pytest

import socialguard_models.transcription as transcription
from socialguard_models.transcription import (
    CompletedOutcome,
    FailedOutcome,
    ModelType,
    TranscriptionTask,
    run_transcription,
)


@pytest.fixture
def task() -> TranscriptionTask:
    return TranscriptionTask(
        model=ModelType.GRANITE,
        audio_uri="s3://example/audio.wav",
        idempotency_key="request-123",
        pipeline_task_id="task-123",
        task_token="token-123",
    )


@pytest.fixture
def services(monkeypatch: pytest.MonkeyPatch) -> Mock:
    services = Mock()
    services.presign.return_value = "https://example.com/audio.wav"
    services.granite.return_value.transcribe.spawn.return_value = services.worker
    services.worker.get.return_value = "A complete transcript."
    monkeypatch.setattr(transcription, "assume_modal_oidc_role", services.authenticate)
    monkeypatch.setattr(transcription, "create_presigned_download_url", services.presign)
    monkeypatch.setattr(transcription, "GraniteSpeech", services.granite)
    monkeypatch.setattr(transcription, "monotonic", Mock(side_effect=[100, 102]))
    monkeypatch.setattr(transcription, "WORKER_TIMEOUT_SECONDS", 42)
    return services


def test_transcription_resolves_s3_and_waits_for_selected_worker(
    task: TranscriptionTask, services: Mock, caplog: pytest.LogCaptureFixture,
) -> None:
    with caplog.at_level(logging.INFO):
        outcome = run_transcription(task)

    assert outcome == CompletedOutcome(text="A complete transcript.")
    services.authenticate.assert_called_once_with()
    services.presign.assert_called_once_with(services.authenticate.return_value, task.audio_uri)
    services.granite.return_value.transcribe.spawn.assert_called_once_with(
        "https://example.com/audio.wav"
    )
    services.worker.get.assert_called_once_with(timeout=40)
    services.worker.cancel.assert_not_called()
    assert "completed" in caplog.text
    assert task.pipeline_task_id in caplog.text
    assert outcome.text not in caplog.text
    assert task.task_token not in caplog.text


@pytest.mark.parametrize("stage", ["authenticate", "presign", "spawn", "get"])
def test_transcription_handles_failures(
    task: TranscriptionTask, services: Mock, stage: str,
    caplog: pytest.LogCaptureFixture,
) -> None:
    operation = {
        "authenticate": services.authenticate,
        "presign": services.presign,
        "spawn": services.granite.return_value.transcribe.spawn,
        "get": services.worker.get,
    }[stage]
    operation.side_effect = RuntimeError("private failure details")

    assert run_transcription(task) == FailedOutcome(error_type="RuntimeError")

    if stage == "get":
        services.worker.cancel.assert_called_once_with()
    else:
        services.worker.cancel.assert_not_called()
        services.worker.get.assert_not_called()
    assert "RuntimeError" in caplog.text
    assert task.pipeline_task_id in caplog.text
    assert "private failure details" not in caplog.text
    assert task.task_token not in caplog.text


def test_transcription_cancels_worker_after_preparation_exhausts_deadline(
    task: TranscriptionTask, services: Mock, monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(transcription, "monotonic", Mock(side_effect=[100, 143]))

    assert run_transcription(task) == FailedOutcome(error_type="TimeoutError")

    services.worker.get.assert_not_called()
    services.worker.cancel.assert_called_once_with()


def test_transcription_preserves_failure_when_cancellation_fails(
    task: TranscriptionTask, services: Mock,
) -> None:
    services.worker.get.side_effect = TimeoutError("worker timed out")
    services.worker.cancel.side_effect = RuntimeError("cancellation failed")

    assert run_transcription(task) == FailedOutcome(error_type="TimeoutError")
    services.worker.cancel.assert_called_once_with()


def test_transcription_rejects_unsupported_model(services: Mock) -> None:
    task = TranscriptionTask(
        model="unsupported",  # type: ignore[arg-type]
        audio_uri="s3://example/audio.wav",
        idempotency_key="request-123",
        pipeline_task_id="task-123",
        task_token="token-123",
    )

    assert run_transcription(task) == FailedOutcome(error_type="ValueError")
    services.granite.assert_not_called()
