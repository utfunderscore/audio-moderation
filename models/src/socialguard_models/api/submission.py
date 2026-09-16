"""Shared HTTP scheduling and acceptance for all model families."""

from fastapi import HTTPException, status

from socialguard_models.api.contracts import QueuedResponse
from socialguard_models.idempotency import SchedulingUnavailableError, schedule_idempotently
from socialguard_models.model_job import ModelJob
from socialguard_models.orchestration import process_model


def submit_model[Result](job: ModelJob[Result]) -> QueuedResponse:
    """Claim work, spawn the shared CPU pipeline, and return without awaiting GPU work."""
    def dispatch(task_id: str) -> None:
        process_model.spawn(job, task_id)  # pyright: ignore[reportFunctionMemberAccess]

    try:
        task_id = schedule_idempotently(
            job.task.idempotency_key, dispatch, family=job.family
        )
    except SchedulingUnavailableError as error:
        raise HTTPException(
            status_code=status.HTTP_503_SERVICE_UNAVAILABLE,
            detail=f"unable to schedule {job.family}",
        ) from error
    return QueuedResponse(task_id=task_id)
