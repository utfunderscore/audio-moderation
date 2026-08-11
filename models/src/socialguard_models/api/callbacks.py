"""Exact-byte HMAC callback construction and bounded retry policy."""

import hashlib
import hmac
import json
import time
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol

CALLBACK_ATTEMPTS = 3
CALLBACK_RETRY_WINDOW_SECONDS = 3
HTTP_SUCCESS_MIN = 200
HTTP_SUCCESS_MAX = 300


class CallbackDeliveryError(RuntimeError):
    """A persisted terminal callback was not acknowledged in this worker attempt."""


@dataclass(frozen=True, slots=True)
class CallbackSigner:
    """Sign a timestamp and immutable callback body."""

    key: bytes
    key_id: str

    def headers(self, body: bytes, timestamp: int | None = None) -> dict[str, str]:
        """Return headers whose HMAC authenticates timestamp and exact body bytes."""
        issued_at = int(time.time()) if timestamp is None else timestamp
        signing_input = str(issued_at).encode("ascii") + b"." + body
        signature = hmac.new(self.key, signing_input, hashlib.sha256).hexdigest()
        return {
            "X-SocialGuard-Timestamp": str(issued_at),
            "X-SocialGuard-Webhook-Key-Id": self.key_id,
            "X-SocialGuard-Signature": f"sha256={signature}",
            "Content-Type": "application/json",
        }


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
    """Serialize a terminal event once so retries sign identical bytes."""
    return json.dumps(event, separators=(",", ":"), sort_keys=True).encode()


async def deliver_with_retries(  # noqa: PLR0913 - narrow testable transport seam.
    sender: CallbackSender,
    url: str,
    body: bytes,
    signer: CallbackSigner,
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
            status_code = await sender.send(url, body, signer.headers(body))
        except Exception:  # noqa: BLE001 - transport failures are retryable.
            status_code = 0
        if HTTP_SUCCESS_MIN <= status_code < HTTP_SUCCESS_MAX:
            return CallbackDelivery(delivered=True, attempts=attempt)
        if attempt < attempts and sleep is not None:
            await sleep(float(2 ** (attempt - 1)))
    message = f"callback was not acknowledged after {attempts} attempts"
    raise CallbackDeliveryError(message)
