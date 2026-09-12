from concurrent.futures import ThreadPoolExecutor
from threading import Event, Lock

import pytest
from fastapi import HTTPException
from pydantic import ValidationError

import socialguard_models.idempotency as idempotency
import socialguard_models.api.transcription_api as transcription_api
from socialguard_models.api.transcription_api import TranscriptionRequest
from socialguard_models.transcription_contracts import ModelType, TranscriptionTask


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


class FinalWriteFailsTaskIds(TaskIds):
    def __init__(self) -> None:
        super().__init__()
        self.fail_scheduled_write = True

    def put(self, key: str, value: object, *, skip_if_exists: bool = False) -> bool:
        if getattr(value, "status", None) == "scheduled" and self.fail_scheduled_write:
            self.fail_scheduled_write = False
            raise RuntimeError("Modal Dict is unavailable")
        return super().put(key, value, skip_if_exists=skip_if_exists)


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


class LateDispatcherScheduler(Scheduler):
    def __init__(self) -> None:
        super().__init__()
        self.lock = Lock()
        self.first_started = Event()
        self.release_first = Event()

    def spawn(self, task: TranscriptionTask, task_id: str) -> None:
        with self.lock:
            call_index = len(self.calls)
            self.calls.append((task, task_id))
        if call_index == 0:
            self.first_started.set()
            assert self.release_first.wait(timeout=5)
        elif call_index == 1:
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


def test_concurrent_retry_does_not_steal_live_claim(
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

    assert all(isinstance(outcome, HTTPException) for outcome in outcomes)
    assert len(scheduler.calls) == 1
    stored = task_ids.values[transcription_request.idempotency_key]
    assert getattr(stored, "task_id") == scheduler.calls[0][1]


def test_transcribe_retains_claim_when_scheduling_fails(
    monkeypatch: pytest.MonkeyPatch, transcription_request: TranscriptionRequest,
) -> None:
    now = [100.0]
    task_ids = TaskIds()
    failing_scheduler = Scheduler(RuntimeError("Modal is unavailable"))
    monkeypatch.setattr(idempotency, "task_ids", task_ids)
    monkeypatch.setattr(idempotency, "time", lambda: now[0])
    monkeypatch.setattr(idempotency, "SCHEDULING_LEASE_SECONDS", 10)
    monkeypatch.setattr(idempotency, "SCHEDULING_RESOLUTION_ATTEMPTS", 1)
    monkeypatch.setattr(transcription_api, "process_transcription", failing_scheduler)

    with pytest.raises(HTTPException) as error:
        transcription_api.transcribe(transcription_request)

    assert error.value.status_code == 503
    stored = task_ids.values[transcription_request.idempotency_key]
    original_task_id = getattr(stored, "task_id")

    now[0] = 111.0
    scheduler = Scheduler()
    monkeypatch.setattr(transcription_api, "process_transcription", scheduler)
    response = transcription_api.transcribe(transcription_request)

    assert response.task_id == original_task_id
    assert scheduler.calls[0][1] == original_task_id


def test_retry_recovers_claim_after_final_write_fails(
    monkeypatch: pytest.MonkeyPatch, transcription_request: TranscriptionRequest,
) -> None:
    now = [100.0]
    task_ids = FinalWriteFailsTaskIds()
    scheduler = Scheduler()
    monkeypatch.setattr(idempotency, "task_ids", task_ids)
    monkeypatch.setattr(idempotency, "time", lambda: now[0], raising=False)
    monkeypatch.setattr(idempotency, "SCHEDULING_LEASE_SECONDS", 10, raising=False)
    monkeypatch.setattr(idempotency, "SCHEDULING_RESOLUTION_ATTEMPTS", 1)
    monkeypatch.setattr(transcription_api, "process_transcription", scheduler)

    with pytest.raises(HTTPException) as error:
        transcription_api.transcribe(transcription_request)
    assert error.value.status_code == 503

    now[0] = 111.0
    response = transcription_api.transcribe(transcription_request)

    assert response.task_id == scheduler.calls[0][1]
    assert [task_id for _, task_id in scheduler.calls] == [
        response.task_id,
        response.task_id,
    ]


def test_retry_recovers_expired_claim_left_before_dispatch(
    monkeypatch: pytest.MonkeyPatch, transcription_request: TranscriptionRequest,
) -> None:
    task_ids = TaskIds()
    task_ids.values[transcription_request.idempotency_key] = (
        idempotency.TaskSchedule(
            task_id="transcription_abandoned",
            status="pending",
            lease_expires_at=100.0,
        )
    )
    scheduler = Scheduler()
    monkeypatch.setattr(idempotency, "task_ids", task_ids)
    monkeypatch.setattr(idempotency, "time", lambda: 101.0)
    monkeypatch.setattr(idempotency, "SCHEDULING_RESOLUTION_ATTEMPTS", 1)
    monkeypatch.setattr(transcription_api, "process_transcription", scheduler)

    response = transcription_api.transcribe(transcription_request)

    assert response.task_id == "transcription_abandoned"
    assert scheduler.calls[0][1] == response.task_id


@pytest.mark.parametrize("status", ["pending", "scheduled"])
def test_retry_normalizes_legacy_schedule_records(
    monkeypatch: pytest.MonkeyPatch,
    transcription_request: TranscriptionRequest,
    status: str,
) -> None:
    task_ids = TaskIds()
    task_ids.values[transcription_request.idempotency_key] = (
        idempotency._TaskSchedule(  # pyright: ignore[reportPrivateUsage]
            task_id="transcription_legacy",
            status=status,  # type: ignore[arg-type]
        )
    )
    scheduler = Scheduler()
    monkeypatch.setattr(idempotency, "task_ids", task_ids)
    monkeypatch.setattr(transcription_api, "process_transcription", scheduler)

    response = transcription_api.transcribe(transcription_request)

    assert response.task_id == "transcription_legacy"
    assert len(scheduler.calls) == (1 if status == "pending" else 0)


def test_late_dispatcher_does_not_overwrite_newer_scheduled_claim(
    monkeypatch: pytest.MonkeyPatch, transcription_request: TranscriptionRequest,
) -> None:
    now = [100.0]
    task_ids = TaskIds()
    scheduler = LateDispatcherScheduler()
    monkeypatch.setattr(idempotency, "task_ids", task_ids)
    monkeypatch.setattr(idempotency, "time", lambda: now[0])
    monkeypatch.setattr(idempotency, "SCHEDULING_LEASE_SECONDS", 10)
    monkeypatch.setattr(transcription_api, "process_transcription", scheduler)

    with ThreadPoolExecutor(max_workers=1) as executor:
        late_response = executor.submit(
            transcription_api.transcribe, transcription_request
        )
        assert scheduler.first_started.wait(timeout=5)
        now[0] = 111.0
        try:
            with pytest.raises(HTTPException):
                transcription_api.transcribe(transcription_request)
            now[0] = 122.0
            current_response = transcription_api.transcribe(transcription_request)
        finally:
            scheduler.release_first.set()
        resolved_late_response = late_response.result(timeout=5)

    assert resolved_late_response.task_id == current_response.task_id
    stored = task_ids.values[transcription_request.idempotency_key]
    assert getattr(stored, "task_id") == current_response.task_id
    assert scheduler.calls[0][1] == scheduler.calls[1][1]
    assert scheduler.calls[1][1] == scheduler.calls[2][1]
    assert scheduler.calls[2][1] == current_response.task_id


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
