import asyncio
import logging
import pickle
from unittest.mock import AsyncMock, Mock

import pytest

import socialguard_models.orchestration as pipeline
import socialguard_models.transcription.job as transcription_job
from socialguard_models.orchestration import run_model
from socialguard_models.transcription.job import TranscriptionJob
from socialguard_models.transcription.contracts import (
    FailedOutcome,
    ModelType,
    TranscriptionTask,
)
from socialguard_models.api import submission
from socialguard_models.moderation.contracts import (
    ModerationScores,
    ModerationTask,
)
from socialguard_models.moderation.job import ModerationJob
from socialguard_models.moderation.models.registry import MODEL_SUBMITTERS


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
    monkeypatch.setattr(pipeline, "assume_modal_oidc_role", services.authenticate)
    monkeypatch.setattr(pipeline, "create_presigned_download_url", services.presign)
    monkeypatch.setattr(transcription_job, "GraniteSpeech", services.granite)
    monkeypatch.setattr(pipeline, "monotonic", Mock(side_effect=[100, 102]))
    monkeypatch.setattr(pipeline, "WORKER_TIMEOUT_SECONDS", 42)
    return services


def test_transcription_resolves_s3_and_waits_for_selected_worker(
    task: TranscriptionTask, services: Mock, caplog: pytest.LogCaptureFixture,
) -> None:
    with caplog.at_level(logging.INFO):
        outcome = asyncio.run(run_model(TranscriptionJob(task)))

    assert outcome == "A complete transcript."
    services.authenticate.assert_called_once_with()
    services.presign.assert_called_once_with(services.authenticate.return_value, task.audio_uri)
    services.granite.return_value.transcribe.spawn.aio.assert_awaited_once_with(
        "https://example.com/audio.wav"
    )
    services.worker.get.aio.assert_awaited_once_with(timeout=40)
    services.worker.cancel.aio.assert_not_awaited()
    assert "completed" in caplog.text
    assert task.pipeline_task_id in caplog.text
    assert outcome not in caplog.text
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

    assert asyncio.run(run_model(TranscriptionJob(task))) == FailedOutcome(
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
    monkeypatch.setattr(pipeline, "monotonic", Mock(side_effect=[100, 143]))

    assert asyncio.run(run_model(TranscriptionJob(task))) == FailedOutcome(
        cause="TimeoutError",
    )

    services.worker.get.aio.assert_not_awaited()
    services.worker.cancel.aio.assert_awaited_once_with()


def test_transcription_preserves_failure_when_cancellation_fails(
    task: TranscriptionTask, services: Mock,
) -> None:
    services.worker.get.aio.side_effect = TimeoutError("worker timed out")
    services.worker.cancel.aio.side_effect = RuntimeError("cancellation failed")

    assert asyncio.run(run_model(TranscriptionJob(task))) == FailedOutcome(
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

    assert asyncio.run(run_model(TranscriptionJob(task))) == FailedOutcome(
        cause="ValueError",
    )
    services.granite.assert_not_called()


def test_process_transcription_posts_worker_outcome(
    task: TranscriptionTask,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    outcome = "A complete transcript."
    job = TranscriptionJob(task)
    run = AsyncMock(return_value=outcome)
    callback = Mock()
    monkeypatch.setattr(
        pipeline,
        "get_callback_uri",
        Mock(return_value="https://example.com/callback"),
    )
    monkeypatch.setattr(pipeline, "run_model", run)
    monkeypatch.setattr(pipeline, "post_callback", callback)
    session = Mock()
    monkeypatch.setattr(pipeline, "assume_modal_oidc_role", Mock(return_value=session))

    asyncio.run(
        pipeline.process_model.local(  # pyright: ignore[reportFunctionMemberAccess]
            job, "transcription-456"
        )
    )

    run.assert_awaited_once_with(job, session)
    callback.assert_called_once_with(
        session=session,
        callback_uri="https://example.com/callback",
        task_token="token-123",
        outcome={
            "type": "success",
            "transcriptionResult": {
                "jobId": "task-123",
                "asrTaskId": "transcription-456",
                "transcription": outcome,
            },
        },
    )


def test_process_transcription_validates_callback_before_worker(
    task: TranscriptionTask,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    run = AsyncMock()
    monkeypatch.setattr(
        pipeline,
        "get_callback_uri",
        Mock(side_effect=RuntimeError("CALLBACK_URI is required")),
    )
    monkeypatch.setattr(pipeline, "run_model", run)

    with pytest.raises(RuntimeError, match="CALLBACK_URI"):
        asyncio.run(
            pipeline.process_model.local(  # pyright: ignore[reportFunctionMemberAccess]
                TranscriptionJob(task), "transcription-456"
            )
        )

    run.assert_not_awaited()


@pytest.mark.parametrize("fails", [False, True])
def test_moderation_uses_same_cpu_gpu_and_callback_lifecycle(
    services: Mock, monkeypatch: pytest.MonkeyPatch, fails: bool,
) -> None:
    task = ModerationTask(
        model="example",
        transcription="Private text to moderate.",
        audio_uri="s3://example/audio.wav",
        idempotency_key="request-123",
        pipeline_task_id="task-123",
        task_token="token-123",
    )
    result: ModerationScores = {
        "sexual": 0.01,
        "hate_or_discrimination": 0.02,
        "harassment_or_abuse": 0.9,
        "violence_or_threats": 0.03,
        "asking_for_pii": 0.04,
    }
    services.worker.get.aio.return_value = result
    if fails:
        services.worker.get.aio.side_effect = TimeoutError("private details")
    submitter = AsyncMock(return_value=services.worker)
    monkeypatch.setitem(MODEL_SUBMITTERS, task.model, submitter)
    job = ModerationJob(task)
    monkeypatch.setenv("CALLBACK_URI", "https://example.com/moderation-callback")
    callback = Mock()
    monkeypatch.setattr(pipeline, "post_callback", callback)

    asyncio.run(pipeline.process_model.local(job, "moderation-456"))

    services.authenticate.assert_called_once_with()
    services.presign.assert_called_once_with(services.authenticate.return_value, task.audio_uri)
    submitter.assert_awaited_once_with(
        "https://example.com/audio.wav", task.transcription
    )
    services.worker.get.aio.assert_awaited_once_with(timeout=40)
    expected = (
        {"type": "failure", "error": "ModerationFailed", "cause": "TimeoutError"}
        if fails else
        {
            "type": "success",
            "moderationResult": {
                "jobId": "task-123",
                "moderationTaskId": "moderation-456",
                "scores": result,
            },
        }
    )
    callback.assert_called_once_with(
        session=services.authenticate.return_value,
        callback_uri="https://example.com/moderation-callback",
        task_token=task.task_token,
        outcome=expected,
    )
    if fails:
        services.worker.cancel.aio.assert_awaited_once_with()
    else:
        services.worker.cancel.aio.assert_not_awaited()


def test_shared_submission_dispatches_moderation_to_same_cpu_function(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    task = ModerationTask(
        model="example",
        transcription="Text to moderate.",
        audio_uri="s3://example/audio.wav",
        idempotency_key="request-123",
        pipeline_task_id="task-123",
        task_token="token-123",
    )
    job = ModerationJob(task)
    scheduler = Mock()
    monkeypatch.setattr(submission, "process_model", scheduler)

    def schedule(key, dispatch, *, family):
        assert key == task.idempotency_key
        assert family == "moderation"
        dispatch("moderation-456")
        return "moderation-456"

    monkeypatch.setattr(submission, "schedule_idempotently", schedule)
    response = submission.submit_model(job)
    assert response.task_id == "moderation-456"
    assert response.status == "queued"
    scheduler.spawn.assert_called_once_with(job, "moderation-456")


def test_transcription_job_round_trips_for_cpu_dispatch(task: TranscriptionTask) -> None:
    job = TranscriptionJob(task)
    assert pickle.loads(pickle.dumps(job)) == job


def test_moderation_job_round_trips_and_rejects_unknown_model(services: Mock) -> None:
    task = ModerationTask(
        model="unconfigured-model",
        transcription="Words spoken in the audio.",
        audio_uri="s3://example/audio.wav",
        idempotency_key="request-123",
        pipeline_task_id="task-123",
        task_token="token-123",
    )
    job = ModerationJob(task)
    restored = pickle.loads(pickle.dumps(job))
    assert restored == job
    assert asyncio.run(run_model(restored)) == FailedOutcome(cause="ValueError")
    services.worker.get.aio.assert_not_awaited()
