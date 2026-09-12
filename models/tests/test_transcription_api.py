from concurrent.futures import ThreadPoolExecutor
from threading import Event, Lock

import pytest
from fastapi import HTTPException
from pydantic import ValidationError

import socialguard_models.idempotency as idempotency
import socialguard_models.api.transcription_api as transcription_api
from socialguard_models.api.transcription_api import TranscriptionRequest
from socialguard_models.transcription import ModelType, TranscriptionTask


class TaskIds:
    def __init__(self, pending_observed: Event | None = None) -> None:
        self.values: dict[str, object] = {}
        self.lock = Lock()
        self.pending_observed = pending_observed

    def put(self, key: str, value: object, *, skip_if_exists: bool = False) -> bool:
        with self.lock:
            if skip_if_exists and key in self.values:
                return False
            self.values[key] = value
            return True

    def get(self, key: str) -> object | None:
        with self.lock:
            value = self.values.get(key)
        if (
            self.pending_observed is not None
            and getattr(value, "status", None) == "pending"
        ):
            self.pending_observed.set()
        return value

    def pop(self, key: str, default: object) -> str | object:
        with self.lock:
            return self.values.pop(key, default)


class Scheduler:
    def __init__(self, failure: Exception | None = None) -> None:
        self.failure = failure
        self.calls: list[tuple[TranscriptionTask, str]] = []

    def spawn(self, task: TranscriptionTask, task_id: str) -> None:
        if self.failure is not None:
            raise self.failure
        self.calls.append((task, task_id))


class FirstAttemptFailsScheduler(Scheduler):
    def __init__(self) -> None:
        super().__init__()
        self.first_started = Event()
        self.release_first = Event()

    def spawn(self, task: TranscriptionTask, task_id: str) -> None:
        self.calls.append((task, task_id))
        if len(self.calls) == 1:
            self.first_started.set()
            assert self.release_first.wait(timeout=5)
            raise RuntimeError("Modal is unavailable")


@pytest.fixture
def transcription_request() -> TranscriptionRequest:
    return TranscriptionRequest(
        model="granite",
        audio_uri="s3://example/audio.wav",
        idempotency_key="request-123",
        pipeline_task_id="task-123",
        task_token="token-123",
    )


def test_transcribe_schedules_once_and_replays_task_id(
    monkeypatch: pytest.MonkeyPatch, transcription_request: TranscriptionRequest,
) -> None:
    task_ids = TaskIds()
    scheduler = Scheduler()
    monkeypatch.setattr(idempotency, "task_ids", task_ids)
    monkeypatch.setattr(transcription_api, "process_transcription", scheduler)

    first = transcription_api.transcribe(transcription_request)
    second = transcription_api.transcribe(transcription_request)

    assert first.status == "queued"
    assert first.task_id.startswith("transcription_")
    assert second == first
    assert scheduler.calls == [
        (
            TranscriptionTask(
                model=ModelType.GRANITE,
                audio_uri="s3://example/audio.wav",
                idempotency_key="request-123",
                pipeline_task_id="task-123",
                task_token="token-123",
            ),
            first.task_id,
        )
    ]


def test_concurrent_retries_schedule_only_the_atomic_claimant(
    monkeypatch: pytest.MonkeyPatch, transcription_request: TranscriptionRequest,
) -> None:
    task_ids = TaskIds()
    scheduler = Scheduler()
    monkeypatch.setattr(idempotency, "task_ids", task_ids)
    monkeypatch.setattr(transcription_api, "process_transcription", scheduler)

    with ThreadPoolExecutor(max_workers=2) as executor:
        responses = list(
            executor.map(
                lambda _: transcription_api.transcribe(transcription_request),
                range(2),
            )
        )

    assert responses[0].task_id == responses[1].task_id
    assert len(scheduler.calls) == 1


def test_retry_waits_for_pending_schedule_before_returning_a_task_id(
    monkeypatch: pytest.MonkeyPatch, transcription_request: TranscriptionRequest,
) -> None:
    pending_observed = Event()
    task_ids = TaskIds(pending_observed)
    scheduler = FirstAttemptFailsScheduler()
    monkeypatch.setattr(idempotency, "task_ids", task_ids)
    monkeypatch.setattr(transcription_api, "process_transcription", scheduler)

    def submit() -> transcription_api.QueuedTranscriptionResponse | HTTPException:
        try:
            return transcription_api.transcribe(transcription_request)
        except HTTPException as error:
            return error

    with ThreadPoolExecutor(max_workers=2) as executor:
        first = executor.submit(submit)
        assert scheduler.first_started.wait(timeout=5)
        second = executor.submit(submit)
        assert pending_observed.wait(timeout=5)
        scheduler.release_first.set()
        outcomes = [first.result(timeout=5), second.result(timeout=5)]

    assert sum(isinstance(outcome, HTTPException) for outcome in outcomes) == 1
    queued = next(
        outcome
        for outcome in outcomes
        if isinstance(outcome, transcription_api.QueuedTranscriptionResponse)
    )
    assert queued.task_id == scheduler.calls[1][1]


def test_transcribe_releases_claim_when_scheduling_fails(
    monkeypatch: pytest.MonkeyPatch, transcription_request: TranscriptionRequest,
) -> None:
    task_ids = TaskIds()
    monkeypatch.setattr(idempotency, "task_ids", task_ids)
    monkeypatch.setattr(
        transcription_api,
        "process_transcription",
        Scheduler(RuntimeError("Modal is unavailable")),
    )

    with pytest.raises(HTTPException) as error:
        transcription_api.transcribe(transcription_request)

    assert error.value.status_code == 503
    assert task_ids.values == {}


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
