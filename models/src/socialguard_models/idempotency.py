"""Shared atomic idempotency handling for asynchronously scheduled work."""

from collections.abc import Callable
from dataclasses import dataclass
from threading import Lock
from time import sleep, time
from typing import Literal
from uuid import uuid4

import modal

task_ids = modal.Dict.from_name(
    "socialguard-transcription-task-ids",
    create_if_missing=True,
)
SCHEDULING_RESOLUTION_ATTEMPTS = 20
SCHEDULING_RESOLUTION_INTERVAL_SECONDS = 0.05
SCHEDULING_LEASE_SECONDS = 30
_claim_locks = tuple(Lock() for _ in range(64))

type ModelFamily = Literal["transcription", "moderation"]
type SchedulingKey = str | tuple[ModelFamily, str]


class SchedulingUnavailableError(RuntimeError):
    """A task could not be scheduled or its scheduling state could not be resolved."""


@dataclass(frozen=True, slots=True)
class _TaskSchedule:
    """Legacy scheduling record retained for persisted Modal Dict values."""

    task_id: str
    status: Literal["pending", "scheduled"]


@dataclass(frozen=True, slots=True)
class TaskSchedule:
    """The shared scheduling state for one idempotency key."""

    task_id: str
    status: Literal["pending", "scheduled"]
    lease_expires_at: float | None = None


@dataclass(frozen=True, slots=True)
class _Claim:
    schedule: TaskSchedule
    acquired: bool


def _claim_lock_for(idempotency_key: SchedulingKey) -> Lock:
    return _claim_locks[hash(idempotency_key) % len(_claim_locks)]


def _normalize_schedule(value: object) -> TaskSchedule | None:
    if isinstance(value, TaskSchedule):
        return value
    if isinstance(value, _TaskSchedule):
        return TaskSchedule(task_id=value.task_id, status=value.status)
    return None


def _claim_or_read(
    idempotency_key: SchedulingKey, claim_lock: Lock, family: ModelFamily
) -> _Claim | None:
    with claim_lock:
        stored = task_ids.get(idempotency_key)
        schedule = _normalize_schedule(stored)
        if schedule is not None and schedule.status == "scheduled":
            return _Claim(schedule=schedule, acquired=False)
        if (
            schedule is not None
            and schedule.lease_expires_at is not None
            and schedule.lease_expires_at > time()
        ):
            return _Claim(schedule=schedule, acquired=False)

        if schedule is not None:
            task_id = schedule.task_id
        elif stored is None:
            task_id = f"{family}_{uuid4().hex}"
        else:
            return None

        pending = TaskSchedule(
            task_id=task_id,
            status="pending",
            lease_expires_at=time() + SCHEDULING_LEASE_SECONDS,
        )
        acquired = task_ids.put(
            idempotency_key,
            pending,
            skip_if_exists=stored is None,
        )
        return _Claim(schedule=pending, acquired=acquired) if acquired else None


def _finalize_if_owned(
    idempotency_key: SchedulingKey,
    schedule: TaskSchedule,
    claim_lock: Lock,
) -> bool:
    with claim_lock:
        if task_ids.get(idempotency_key) != schedule:
            return False
        task_ids.put(
            idempotency_key,
            TaskSchedule(task_id=schedule.task_id, status="scheduled"),
        )
        return True


def _wait_before_retry(attempt: int) -> None:
    if attempt < SCHEDULING_RESOLUTION_ATTEMPTS - 1:
        sleep(SCHEDULING_RESOLUTION_INTERVAL_SECONDS)


def schedule_idempotently(
    idempotency_key: str,
    dispatch: Callable[[str], None],
    *,
    family: ModelFamily = "transcription",
) -> str:
    """Schedule work once per family/key, retaining existing transcription claims."""
    key: SchedulingKey = (
        idempotency_key if family == "transcription" else (family, idempotency_key)
    )
    claim_lock = _claim_lock_for(key)
    for attempt in range(SCHEDULING_RESOLUTION_ATTEMPTS):
        claim = _claim_or_read(key, claim_lock, family)
        if claim is not None and claim.schedule.status == "scheduled":
            return claim.schedule.task_id

        if claim is not None and claim.acquired:
            try:
                dispatch(claim.schedule.task_id)
                finalized = _finalize_if_owned(
                    key,
                    claim.schedule,
                    claim_lock,
                )
            except Exception as error:
                raise SchedulingUnavailableError from error
            if finalized:
                return claim.schedule.task_id

        _wait_before_retry(attempt)

    raise SchedulingUnavailableError
