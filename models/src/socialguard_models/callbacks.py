"""Shared SigV4 callback transport and retry policy."""

import json
import os
from time import sleep
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

import boto3
from botocore.auth import SigV4Auth
from botocore.awsrequest import AWSRequest

CALLBACK_REQUEST_TIMEOUT_SECONDS = 10
CALLBACK_MAX_ATTEMPTS = 3
CALLBACK_INITIAL_BACKOFF_SECONDS = 1
CALLBACK_MAX_DURATION_SECONDS = (
    CALLBACK_REQUEST_TIMEOUT_SECONDS * CALLBACK_MAX_ATTEMPTS
    + CALLBACK_INITIAL_BACKOFF_SECONDS * (2 ** (CALLBACK_MAX_ATTEMPTS - 1) - 1)
)
CALLBACK_URI_ENVIRONMENT_VARIABLE = "CALLBACK_URI"


def get_callback_uri() -> str:
    """Return the configured callback URI or fail before work begins."""
    callback_uri = os.environ.get(CALLBACK_URI_ENVIRONMENT_VARIABLE, "").strip()
    if not callback_uri:
        raise RuntimeError(f"{CALLBACK_URI_ENVIRONMENT_VARIABLE} is required")
    return callback_uri


def post_callback(
    *,
    session: boto3.Session,
    callback_uri: str,
    task_token: str,
    outcome: dict[str, object],
) -> None:
    """Post a SigV4-authenticated terminal outcome, retrying transient failures."""
    region = os.environ.get("AWS_REGION", "").strip()
    if not region:
        raise RuntimeError("AWS_REGION is not configured")
    payload = json.dumps({"taskToken": task_token, "outcome": outcome}).encode(
        "utf-8"
    )
    credentials = session.get_credentials()
    if credentials is None:
        raise RuntimeError("AWS credentials are not available")
    signed_request = AWSRequest(
        method="POST",
        url=callback_uri,
        data=payload,
        headers={"Content-Type": "application/json"},
    )
    SigV4Auth(credentials.get_frozen_credentials(), "execute-api", region).add_auth(
        signed_request
    )
    request = Request(
        callback_uri,
        data=payload,
        headers=dict(signed_request.headers.items()),
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
