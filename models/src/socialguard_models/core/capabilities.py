"""Capabilities advertised by an ASR model deployment."""

from dataclasses import dataclass, field
from enum import StrEnum


class ResponseFormat(StrEnum):
    """Response formats understood by the shared transcription API."""

    JSON = "json"


class TranscriptionOption(StrEnum):
    """Optional transcription features that a backend may support."""

    LANGUAGE = "language"
    PROMPT = "prompt"
    RESPONSE_FORMAT = "response_format"


def _default_response_formats() -> frozenset[ResponseFormat]:
    """Return the response formats required by the base API contract."""
    return frozenset({ResponseFormat.JSON})


@dataclass(frozen=True, slots=True)
class ModelCapabilities:
    """Optional features supported by one concrete model deployment."""

    language: bool = False
    prompt: bool = False
    response_formats: frozenset[ResponseFormat] = field(
        default_factory=_default_response_formats,
    )

    def __post_init__(self) -> None:
        """Reject a deployment that cannot produce any response format."""
        if not self.response_formats:
            message = "A model must support at least one response format."
            raise ValueError(message)
