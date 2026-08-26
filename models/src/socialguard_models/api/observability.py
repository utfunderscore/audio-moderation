"""Structured JSON-line event logging for gateway diagnosis."""

import json
import logging
import sys
from urllib.parse import urlsplit

_logger = logging.getLogger("socialguard")
if not _logger.handlers and not logging.getLogger().handlers:
    _handler = logging.StreamHandler(sys.stderr)
    _handler.setFormatter(logging.Formatter("%(message)s"))
    _logger.addHandler(_handler)
_logger.setLevel(logging.INFO)


def redacted_url(url: str) -> str:
    """Return a log-safe URL with any query string replaced."""
    parsed = urlsplit(url)
    if not parsed.query:
        return url
    return parsed._replace(query="REDACTED").geturl()


def log_event(
    event: str,
    level: int = logging.INFO,
    /,
    *,
    exc_info: bool = False,
    **fields: object,
) -> None:
    """Emit one parseable JSON log line describing a named gateway event."""
    payload = {"event": event, **fields}
    _logger.log(
        level,
        json.dumps(payload, default=str, separators=(",", ":")),
        exc_info=exc_info,
    )
