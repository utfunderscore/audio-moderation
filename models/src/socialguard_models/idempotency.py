"""Shared atomic idempotency handling for asynchronously scheduled work."""

from collections.abc import Callable
from dataclasses import dataclass
from time import sleep
from typing import Literal
from uuid import uuid4

import modal

task_ids = modal.Dict.from_name(
    "socialguard-transcription-task-ids",
    create_if_missing=True,
)
SCHEDULING_RESOLUTION_ATTEMPTS = 20
SCHEDULING_RESOLUTION_INTERVAL_SECONDS = 0.05


class SchedulingUnavailableError(RuntimeError):
    """A task could not be scheduled or its scheduling state could not be resolved."""


@dataclass(frozen=True, slots=True)
class _TaskSchedule:
    """The shared scheduling state for one idempotency key."""

    task_id: str
    status: Literal["pending", "scheduled"]


def schedule_idempotently(
    idempotency_key: str,
    dispatch: Callable[[str], None],
) -> str:
    """Schedule work once and return its task ID after the dispatch is accepted."""
    for attempt in range(SCHEDULING_RESOLUTION_ATTEMPTS):
        task_id = f"transcription_{uuid4().hex}"
        pending = _TaskSchedule(task_id=task_id, status="pending")
        if task_ids.put(idempotency_key, pending, skip_if_exists=True):
            try:
                dispatch(task_id)
            except Exception as error:
                task_ids.pop(idempotency_key, None)
                raise SchedulingUnavailableError from error

            try:
                task_ids.put(
                    idempotency_key,
                    _TaskSchedule(task_id=task_id, status="scheduled"),
                )
            except Exception as error:
                raise SchedulingUnavailableError from error
            return task_id

        existing = task_ids.get(idempotency_key)
        if isinstance(existing, _TaskSchedule) and existing.status == "scheduled":
            return existing.task_id
        if attempt < SCHEDULING_RESOLUTION_ATTEMPTS - 1:
            sleep(SCHEDULING_RESOLUTION_INTERVAL_SECONDS)

    raise SchedulingUnavailableError
