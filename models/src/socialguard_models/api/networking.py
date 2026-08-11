"""Small, testable policy helpers for dereferencing caller-supplied HTTPS URLs."""

import ipaddress
import socket
from collections.abc import Callable, Iterable
from dataclasses import dataclass
from urllib.parse import SplitResult, urlsplit

MAX_REMOTE_BYTES = 25 * 1024 * 1024


class UnsafeUrlError(ValueError):
    """A URL is unsuitable for server-side retrieval or callback delivery."""


@dataclass(frozen=True, slots=True)
class NetworkPolicy:
    """Constraints applied before an HTTP client connects to an untrusted host."""

    allowed_hosts: frozenset[str] | None = None


def _is_public_address(address: str) -> bool:
    """Return whether an IPv4/IPv6 address is safe as an Internet destination."""
    value = ipaddress.ip_address(address)
    return not (
        value.is_private
        or value.is_loopback
        or value.is_link_local
        or value.is_multicast
        or value.is_reserved
        or value.is_unspecified
    )


def validate_url(
    value: str,
    policy: NetworkPolicy | None = None,
    resolver: Callable[[str, int], Iterable[str]] | None = None,
) -> SplitResult:
    """Validate syntax and resolved addresses before a network connection.

    Callers must re-run this validation for every redirect. Typical Python HTTP clients
    cannot pin the subsequently connected address, so DNS rebinding remains a residual
    risk unless deployment networking also enforces an egress allowlist.
    """
    active_policy = NetworkPolicy() if policy is None else policy
    parsed = urlsplit(value)
    if (
        parsed.scheme != "https"
        or not parsed.hostname
        or parsed.username is not None
        or parsed.password is not None
        or parsed.fragment
        or parsed.port not in (None, 443)
    ):
        message = "URL must be HTTPS on port 443 without credentials or fragments."
        raise UnsafeUrlError(message)
    hostname = parsed.hostname.rstrip(".").lower()
    if (
        active_policy.allowed_hosts is not None
        and hostname not in active_policy.allowed_hosts
    ):
        message = "URL host is not allowlisted."
        raise UnsafeUrlError(message)
    if resolver is None:
        infos = socket.getaddrinfo(hostname, 443, type=socket.SOCK_STREAM)
        addresses = {info[4][0] for info in infos}
    else:
        addresses = set(resolver(hostname, 443))
    string_addresses = {str(address) for address in addresses}
    if not string_addresses or any(
        not _is_public_address(address) for address in string_addresses
    ):
        message = "URL must resolve exclusively to public addresses."
        raise UnsafeUrlError(message)
    return parsed
