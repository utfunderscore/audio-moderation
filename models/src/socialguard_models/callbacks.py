"""Shared direct-Lambda callback transport and retry policy."""

import json
import os
from time import sleep
from typing import Protocol, cast, runtime_checkable

import boto3
from botocore.config import Config
from botocore.exceptions import BotoCoreError, ClientError

CALLBACK_REQUEST_TIMEOUT_SECONDS = 10
CALLBACK_MAX_ATTEMPTS = 3
CALLBACK_INITIAL_BACKOFF_SECONDS = 1
CALLBACK_MAX_DURATION_SECONDS = (
    CALLBACK_REQUEST_TIMEOUT_SECONDS * CALLBACK_MAX_ATTEMPTS
    + CALLBACK_INITIAL_BACKOFF_SECONDS * (2 ** (CALLBACK_MAX_ATTEMPTS - 1) - 1)
)
CALLBACK_FUNCTION_NAME_ENVIRONMENT_VARIABLE = "TASK_CALLBACK_FUNCTION_NAME"
CALLBACK_PATH = "/callbacks/external-task"


@runtime_checkable
class ResponsePayload(Protocol):
    """The readable response stream returned by synchronous Lambda invocation."""

    def read(self) -> bytes: ...


class CallbackLambdaClient(Protocol):
    """Subset of the Lambda client used for callback delivery."""

    def invoke(
        self,
        *,
        FunctionName: str,
        InvocationType: str,
        Payload: bytes,
    ) -> dict[str, object]: ...


class CallbackDeliveryError(RuntimeError):
    """A callback Lambda invocation or its HTTP-shaped result failed."""

    def __init__(self, status_code: int | None) -> None:
        self.status_code = status_code
        super().__init__(
            "task callback Lambda invocation failed"
            if status_code is None
            else f"task callback Lambda returned HTTP {status_code}"
        )


def get_callback_function_name() -> str:
    """Return the callback Lambda configured for direct invocation."""
    function_name = os.environ.get(CALLBACK_FUNCTION_NAME_ENVIRONMENT_VARIABLE, "").strip()
    if not function_name:
        raise RuntimeError(f"{CALLBACK_FUNCTION_NAME_ENVIRONMENT_VARIABLE} is required")
    return function_name


def callback_event(*, task_token: str, outcome: dict[str, object]) -> bytes:
    """Build the API Gateway v2 envelope expected by the callback Lambda."""
    body = json.dumps({"taskToken": task_token, "outcome": outcome})
    return json.dumps(
        {
            "version": "2.0",
            "routeKey": f"POST {CALLBACK_PATH}",
            "rawPath": CALLBACK_PATH,
            "rawQueryString": "",
            "isBase64Encoded": False,
            "headers": {"content-type": "application/json"},
            "requestContext": {
                "accountId": "modal",
                "apiId": "modal",
                "domainName": "modal",
                "domainPrefix": "modal",
                "requestId": "modal",
                "routeKey": f"POST {CALLBACK_PATH}",
                "stage": "$default",
                "time": "01/Jan/1970:00:00:00 +0000",
                "timeEpoch": 0,
                "http": {
                    "method": "POST",
                    "path": CALLBACK_PATH,
                    "protocol": "HTTP/1.1",
                    "sourceIp": "127.0.0.1",
                    "userAgent": "SocialGuardModalCallback",
                },
            },
            "body": body,
        }
    ).encode("utf-8")


def callback_status(response: dict[str, object]) -> int:
    """Extract the callback Lambda's HTTP status from a synchronous invoke."""
    if response.get("FunctionError"):
        raise CallbackDeliveryError(None)
    payload = response.get("Payload")
    if not isinstance(payload, ResponsePayload):
        raise CallbackDeliveryError(None)
    try:
        result = json.loads(payload.read())
        status_code = result["statusCode"]
    except (KeyError, TypeError, ValueError):
        raise CallbackDeliveryError(None) from None
    if not isinstance(status_code, int):
        raise CallbackDeliveryError(None)
    return status_code


def retryable(error: Exception) -> bool:
    """Classify callback errors without exposing service response bodies."""
    return not isinstance(error, CallbackDeliveryError) or error.status_code in {
        408,
        425,
        429,
        500,
        502,
        503,
    }


def post_callback(
    *,
    session: boto3.Session,
    callback_function_name: str,
    task_token: str,
    outcome: dict[str, object],
) -> None:
    """Invoke the callback Lambda with a terminal outcome, retrying transient failures."""
    region = os.environ.get("AWS_REGION", "").strip()
    if not region:
        raise RuntimeError("AWS_REGION is not configured")
    client = cast(
        CallbackLambdaClient,
        session.client(  # pyright: ignore[reportUnknownMemberType]
            "lambda",
            region_name=region,
            config=Config(
                connect_timeout=CALLBACK_REQUEST_TIMEOUT_SECONDS,
                read_timeout=CALLBACK_REQUEST_TIMEOUT_SECONDS,
                retries={"max_attempts": 0},
            ),
        ),
    )
    payload = callback_event(task_token=task_token, outcome=outcome)
    for attempt in range(CALLBACK_MAX_ATTEMPTS):
        try:
            response = client.invoke(
                FunctionName=callback_function_name,
                InvocationType="RequestResponse",
                Payload=payload,
            )
            status_code = callback_status(response)
            if 200 <= status_code < 300:
                return
            raise CallbackDeliveryError(status_code)
        except (BotoCoreError, ClientError, CallbackDeliveryError) as error:
            if not retryable(error) or attempt == CALLBACK_MAX_ATTEMPTS - 1:
                raise
            sleep(CALLBACK_INITIAL_BACKOFF_SECONDS * 2**attempt)
