"""Exact-byte SigV4 callback construction and bounded retry policy."""

import json
import logging
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol, cast

from botocore.auth import SigV4Auth
from botocore.awsrequest import AWSRequest
from botocore.credentials import Credentials
from pydantic import TypeAdapter

from socialguard_models.api.observability import log_event
from socialguard_models.api.schemas import (
    CallbackEvent,
    TranscriptionCompleted,
    TranscriptionFailed,
)

CALLBACK_ATTEMPTS = 3
CALLBACK_RETRY_WINDOW_SECONDS = 3
HTTP_NO_CONTENT = 204
HTTP_BAD_REQUEST = 400
HTTP_FORBIDDEN = 403


class CallbackDeliveryError(RuntimeError):
    """A persisted terminal callback was not acknowledged in this worker attempt."""


class CallbackContractError(CallbackDeliveryError):
    """The callback receiver rejected the immutable terminal payload."""


class CallbackAuthenticationError(CallbackDeliveryError):
    """AWS rejected the callback sender's OIDC exchange or SigV4 credentials."""


@dataclass(frozen=True, slots=True)
class SigV4CallbackSigner:
    """Sign immutable callback bytes with temporary AWS session credentials."""

    access_key_id: str
    secret_access_key: str
    session_token: str
    region: str
    service: str

    def headers(self, url: str, body: bytes) -> dict[str, str]:
        """Return SigV4 headers including the temporary STS session token."""
        request = AWSRequest(
            method="POST",
            url=url,
            data=body,
            headers={"Content-Type": "application/json"},
        )
        credentials = Credentials(
            self.access_key_id,
            self.secret_access_key,
            self.session_token,
        )
        SigV4Auth(credentials, self.service, self.region).add_auth(request)
        return dict(request.headers.items())


class CallbackRequestSigner(Protocol):
    """Produce authentication headers for one immutable callback request."""

    def headers(self, url: str, body: bytes) -> dict[str, str]:  # noqa: D102
        ...


class CallbackSender(Protocol):
    """Narrow async HTTP seam used by the orchestration worker."""

    async def send(  # noqa: D102
        self, url: str, body: bytes, headers: dict[str, str]
    ) -> int: ...


@dataclass(frozen=True, slots=True)
class CallbackDelivery:
    """Outcome of bounded callback attempts."""

    delivered: bool
    attempts: int


def terminal_body(event: dict[str, object]) -> bytes:
    """Validate and serialize a terminal event once for exact-byte retries."""
    validated = cast(
        "TranscriptionCompleted | TranscriptionFailed",
        TypeAdapter(CallbackEvent).validate_python(event),
    )
    return json.dumps(
        validated.model_dump(mode="json"), separators=(",", ":"), sort_keys=True
    ).encode()


async def deliver_with_retries(  # noqa: PLR0913 - narrow testable transport seam.
    sender: CallbackSender,
    url: str,
    body: bytes,
    signer: CallbackRequestSigner,
    *,
    attempts: int = CALLBACK_ATTEMPTS,
    sleep: Callable[[float], Awaitable[None]] | None = None,
) -> CallbackDelivery:
    """Make bounded attempts; callers retry the persisted body after exhaustion."""
    if attempts < 1:
        message = "attempts must be positive"
        raise ValueError(message)
    for attempt in range(1, attempts + 1):
        try:
            status_code = await sender.send(url, body, signer.headers(url, body))
        except Exception as exc:  # noqa: BLE001 - transport failures are retryable.
            status_code = 0
            log_event(
                "callback_attempt_transport_error",
                level=logging.WARNING,
                attempt=attempt,
                error_type=type(exc).__name__,
                detail=str(exc),
            )
        if status_code == HTTP_NO_CONTENT:
            log_event("callback_delivered", attempt=attempt)
            return CallbackDelivery(delivered=True, attempts=attempt)
        log_event(
            "callback_attempt_rejected",
            level=logging.WARNING,
            attempt=attempt,
            status_code=status_code,
        )
        if status_code == HTTP_BAD_REQUEST:
            message = "callback receiver rejected the terminal payload"
            raise CallbackContractError(message)
        if status_code == HTTP_FORBIDDEN:
            message = "callback receiver rejected AWS authentication"
            raise CallbackAuthenticationError(message)
        if attempt < attempts and sleep is not None:
            await sleep(float(2 ** (attempt - 1)))
    message = f"callback was not acknowledged after {attempts} attempts"
    raise CallbackDeliveryError(message)
