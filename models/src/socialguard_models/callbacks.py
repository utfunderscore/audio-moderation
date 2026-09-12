"""Transcription completion callback contracts and delivery."""

import json
import os
from time import sleep
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

from socialguard_models.transcription_contracts import (
    CompletedOutcome,
    TranscriptionOutcome,
)

CALLBACK_REQUEST_TIMEOUT_SECONDS = 10
CALLBACK_MAX_ATTEMPTS = 3
CALLBACK_INITIAL_BACKOFF_SECONDS = 1
CALLBACK_MAX_DURATION_SECONDS = (
    CALLBACK_REQUEST_TIMEOUT_SECONDS * CALLBACK_MAX_ATTEMPTS
    + CALLBACK_INITIAL_BACKOFF_SECONDS * (2 ** (CALLBACK_MAX_ATTEMPTS - 1) - 1)
)


def get_callback_uri() -> str:
    """Return the configured callback URI or fail before work begins."""
    callback_uri = os.environ.get("TRANSCRIPTION_CALLBACK_URI", "").strip()
    if not callback_uri:
        raise RuntimeError("TRANSCRIPTION_CALLBACK_URI is required")
    return callback_uri


def post_completion_callback(
    *,
    callback_uri: str,
    task_token: str,
    job_id: str,
    asr_task_id: str,
    outcome: TranscriptionOutcome,
) -> None:
    """Post a terminal outcome, retrying transient delivery failures."""
    if isinstance(outcome, CompletedOutcome):
        callback_outcome: dict[str, object] = {
            "type": "success",
            "transcriptionResult": {
                "jobId": job_id,
                "asrTaskId": asr_task_id,
                "transcription": outcome.text,
            },
        }
    else:
        callback_outcome = {
            "type": "failure",
            "error": "TranscriptionFailed",
            "cause": outcome.cause,
        }

    request = Request(
        callback_uri,
        data=json.dumps(
            {"taskToken": task_token, "outcome": callback_outcome}
        ).encode("utf-8"),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    for attempt in range(CALLBACK_MAX_ATTEMPTS):
        try:
            with urlopen(request, timeout=CALLBACK_REQUEST_TIMEOUT_SECONDS) as response:
                response.read()
            return
        except (HTTPError, URLError, TimeoutError) as error:
            retryable = not isinstance(error, HTTPError) or error.code in {
                408,
                425,
                429,
            } or error.code >= 500
            if not retryable or attempt == CALLBACK_MAX_ATTEMPTS - 1:
                raise
            sleep(CALLBACK_INITIAL_BACKOFF_SECONDS * 2**attempt)
