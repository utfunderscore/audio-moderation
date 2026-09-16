"""One CPU/GPU execution lifecycle shared by all model families."""

import asyncio
import logging
from time import monotonic

import boto3
import modal

from socialguard_models.aws import assume_modal_oidc_role, create_presigned_download_url
from socialguard_models.callbacks import (
    CALLBACK_MAX_DURATION_SECONDS,
    get_callback_uri,
    post_callback,
)
from socialguard_models.contracts import FailedOutcome
from socialguard_models.modal_app import app, cpu_image, runtime_secret
from socialguard_models.model_job import ModelJob, SubmittedModel

logger = logging.getLogger(__name__)
WORKER_TIMEOUT_SECONDS = 660
MAX_CONCURRENT_TASKS = 32
PROCESS_TIMEOUT_SECONDS = WORKER_TIMEOUT_SECONDS + CALLBACK_MAX_DURATION_SECONDS


async def run_model[Result](
    job: ModelJob[Result], session: boto3.Session | None = None
) -> Result | FailedOutcome:
    """Resolve audio, submit GPU work, and wait within one preparation/work deadline."""
    deadline = monotonic() + WORKER_TIMEOUT_SECONDS
    worker_call: SubmittedModel[Result] | None = None
    try:
        if session is None:
            session = await asyncio.to_thread(assume_modal_oidc_role)
        audio_url = await asyncio.to_thread(
            create_presigned_download_url, session, job.task.audio_uri
        )
        worker_call = await job.submit(audio_url)
        remaining_seconds = deadline - monotonic()
        if remaining_seconds <= 0:
            raise TimeoutError("model worker deadline exceeded")
        result = await worker_call.get.aio(timeout=remaining_seconds)
    except Exception as error:
        if worker_call is not None:
            try:
                await worker_call.cancel.aio()
            except Exception:
                pass
        logger.warning(
            "%s worker failed: pipeline_task_id=%s model=%s error_type=%s",
            job.family,
            job.task.pipeline_task_id,
            job.model,
            type(error).__name__,
        )
        return FailedOutcome(cause=type(error).__name__)

    logger.info(
        "%s worker completed: pipeline_task_id=%s model=%s",
        job.family,
        job.task.pipeline_task_id,
        job.model,
    )
    return result


@app.function(  # pyright: ignore[reportUnknownMemberType]
    image=cpu_image,
    secrets=[runtime_secret],
    timeout=PROCESS_TIMEOUT_SECONDS,
)
@modal.concurrent(max_inputs=MAX_CONCURRENT_TASKS)  # pyright: ignore[reportUnknownMemberType]
async def process_model[Result](job: ModelJob[Result], task_id: str) -> None:
    """Run every accepted model job outside the HTTP lifecycle on shared CPU compute."""
    callback_uri = get_callback_uri()
    session = await asyncio.to_thread(assume_modal_oidc_role)
    result = await run_model(job, session)
    await asyncio.to_thread(
        post_callback,
        session=session,
        callback_uri=callback_uri,
        task_token=job.task.task_token,
        outcome=job.callback_outcome(result, task_id),
    )
    logger.info(
        "%s task reached a terminal outcome: task_id=%s failed=%s",
        job.family,
        task_id,
        isinstance(result, FailedOutcome),
    )
