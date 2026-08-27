"""Bearer-token authentication for the single-tenant gateway boundary."""

import os
import secrets
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Annotated

from fastapi import Depends
from fastapi.security import HTTPAuthorizationCredentials, HTTPBearer

API_TOKEN_ENVIRONMENT_VARIABLE = "SOCIALGUARD_GATEWAY_API_TOKEN"  # noqa: S105
_SINGLE_TENANT_CALLER_ID = "single-tenant"
_bearer_scheme = HTTPBearer(auto_error=False)


@dataclass(frozen=True, slots=True)
class GatewaySettings:
    """Configuration required by the public gateway HTTP boundary."""

    api_token: str
    caller_id: str = _SINGLE_TENANT_CALLER_ID

    @classmethod
    def from_environment(cls) -> "GatewaySettings":
        """Load the required single-tenant token from the process environment."""
        token = os.environ.get(API_TOKEN_ENVIRONMENT_VARIABLE)
        if not token:
            message = f"{API_TOKEN_ENVIRONMENT_VARIABLE} must be configured."
            raise RuntimeError(message)
        return cls(api_token=token)


def require_caller(
    settings: GatewaySettings,
) -> "CallerDependency":
    """Return a dependency that authenticates the configured bearer token."""

    async def authenticate(
        credentials: Annotated[
            HTTPAuthorizationCredentials | None,
            Depends(_bearer_scheme),
        ],
    ) -> str:
        """Return the authenticated caller identity or reject the request."""
        if credentials is None:
            reason = "missing_bearer_token"
            raise GatewayAuthenticationError(reason)
        if not secrets.compare_digest(credentials.credentials, settings.api_token):
            reason = "invalid_bearer_token"
            raise GatewayAuthenticationError(reason)
        return settings.caller_id

    return authenticate


class GatewayAuthenticationError(Exception):
    """Signal that a request did not present the configured bearer token."""


CallerDependency = Callable[..., Awaitable[str]]
