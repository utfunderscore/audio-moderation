import asyncio
import logging
from unittest.mock import AsyncMock, Mock

import pytest

import socialguard_models.transcription as transcription
from socialguard_models.transcription import run_transcription
from socialguard_models.transcription_contracts import (
    CompletedOutcome,
    FailedOutcome,
    ModelType,
    TranscriptionTask,
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
    services.granite.return_value.transcribe.spawn.aio = AsyncMock(
        return_value=services.worker
    )
    services.worker.get.aio = AsyncMock(return_value="A complete transcript.")
    services.worker.cancel.aio = AsyncMock()
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
        outcome = asyncio.run(run_transcription(task))

    assert outcome == CompletedOutcome(text="A complete transcript.")
    assert isinstance(outcome, CompletedOutcome)
    services.authenticate.assert_called_once_with()
    services.presign.assert_called_once_with(services.authenticate.return_value, task.audio_uri)
    services.granite.return_value.transcribe.spawn.aio.assert_awaited_once_with(
        "https://example.com/audio.wav"
    )
    services.worker.get.aio.assert_awaited_once_with(timeout=40)
    services.worker.cancel.aio.assert_not_awaited()
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
        "spawn": services.granite.return_value.transcribe.spawn.aio,
        "get": services.worker.get.aio,
    }[stage]
    operation.side_effect = RuntimeError("private failure details")

    assert asyncio.run(run_transcription(task)) == FailedOutcome(
        cause="RuntimeError",
    )

    if stage == "get":
        services.worker.cancel.aio.assert_awaited_once_with()
    else:
        services.worker.cancel.aio.assert_not_awaited()
        services.worker.get.aio.assert_not_awaited()
    assert "RuntimeError" in caplog.text
    assert task.pipeline_task_id in caplog.text
    assert "private failure details" not in caplog.text
    assert task.task_token not in caplog.text


def test_transcription_cancels_worker_after_preparation_exhausts_deadline(
    task: TranscriptionTask, services: Mock, monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(transcription, "monotonic", Mock(side_effect=[100, 143]))

    assert asyncio.run(run_transcription(task)) == FailedOutcome(
        cause="TimeoutError",
    )

    services.worker.get.aio.assert_not_awaited()
    services.worker.cancel.aio.assert_awaited_once_with()


def test_transcription_preserves_failure_when_cancellation_fails(
    task: TranscriptionTask, services: Mock,
) -> None:
    services.worker.get.aio.side_effect = TimeoutError("worker timed out")
    services.worker.cancel.aio.side_effect = RuntimeError("cancellation failed")

    assert asyncio.run(run_transcription(task)) == FailedOutcome(
        cause="TimeoutError",
    )
    services.worker.cancel.aio.assert_awaited_once_with()


def test_transcription_rejects_unsupported_model(services: Mock) -> None:
    task = TranscriptionTask(
        model="unsupported",  # type: ignore[arg-type]
        audio_uri="s3://example/audio.wav",
        idempotency_key="request-123",
        pipeline_task_id="task-123",
        task_token="token-123",
    )

    assert asyncio.run(run_transcription(task)) == FailedOutcome(
        cause="ValueError",
    )
    services.granite.assert_not_called()


def test_process_transcription_posts_worker_outcome(
    task: TranscriptionTask,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    outcome = CompletedOutcome(text="A complete transcript.")
    run = AsyncMock(return_value=outcome)
    callback = Mock()
    monkeypatch.setattr(
        transcription,
        "get_callback_uri",
        Mock(return_value="https://example.com/callback"),
    )
    monkeypatch.setattr(transcription, "run_transcription", run)
    monkeypatch.setattr(transcription, "post_completion_callback", callback)

    asyncio.run(
        transcription.process_transcription.local(  # pyright: ignore[reportFunctionMemberAccess]
            task, "transcription-456"
        )
    )

    run.assert_awaited_once_with(task)
    callback.assert_called_once_with(
        callback_uri="https://example.com/callback",
        task_token="token-123",
        job_id="task-123",
        asr_task_id="transcription-456",
        outcome=outcome,
    )


def test_process_transcription_validates_callback_before_worker(
    task: TranscriptionTask,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    run = AsyncMock()
    monkeypatch.setattr(
        transcription,
        "get_callback_uri",
        Mock(side_effect=RuntimeError("TRANSCRIPTION_CALLBACK_URI is required")),
    )
    monkeypatch.setattr(transcription, "run_transcription", run)

    with pytest.raises(RuntimeError, match="TRANSCRIPTION_CALLBACK_URI"):
        asyncio.run(
            transcription.process_transcription.local(  # pyright: ignore[reportFunctionMemberAccess]
                task, "transcription-456"
            )
        )

    run.assert_not_awaited()
