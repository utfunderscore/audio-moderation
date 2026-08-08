"""Model-independent values used by ASR backends."""

from dataclasses import dataclass
from pathlib import Path

from socialguard_models.core.capabilities import (
    ModelCapabilities,
    ResponseFormat,
    TranscriptionOption,
)


def _require_non_empty(value: str, field_name: str) -> None:
    """Reject empty or whitespace-only identifiers."""
    if not value.strip():
        message = f"{field_name} must not be empty."
        raise ValueError(message)


@dataclass(frozen=True, slots=True)
class AudioInput:
    """A request-scoped audio file prepared for an ASR backend."""

    path: Path
    filename: str | None = None
    content_type: str | None = None

    def __post_init__(self) -> None:
        """Validate optional upload metadata without touching the filesystem."""
        if self.filename is not None:
            _require_non_empty(self.filename, "filename")
        if self.content_type is not None:
            _require_non_empty(self.content_type, "content_type")


@dataclass(frozen=True, slots=True)
class TranscriptionOptions:
    """Model-neutral options accepted by the shared transcription API."""

    language: str | None = None
    prompt: str | None = None
    response_format: ResponseFormat = ResponseFormat.JSON

    def __post_init__(self) -> None:
        """Reject present but empty request options."""
        if self.language is not None:
            _require_non_empty(self.language, "language")
        if self.prompt is not None:
            _require_non_empty(self.prompt, "prompt")

    def unsupported_by(
        self,
        capabilities: ModelCapabilities,
    ) -> frozenset[TranscriptionOption]:
        """Return options requested here but unsupported by a model."""
        unsupported: set[TranscriptionOption] = set()
        if self.language is not None and not capabilities.language:
            unsupported.add(TranscriptionOption.LANGUAGE)
        if self.prompt is not None and not capabilities.prompt:
            unsupported.add(TranscriptionOption.PROMPT)
        if self.response_format not in capabilities.response_formats:
            unsupported.add(TranscriptionOption.RESPONSE_FORMAT)
        return frozenset(unsupported)


@dataclass(frozen=True, slots=True)
class Transcript:
    """Portable transcription result returned by every backend."""

    text: str


@dataclass(frozen=True, slots=True)
class ModelInfo:
    """Identity and capabilities of one immutable model deployment."""

    model_id: str
    engine: str
    revision: str
    capabilities: ModelCapabilities
    aliases: tuple[str, ...] = ()

    def __post_init__(self) -> None:
        """Validate identifiers and prevent ambiguous model aliases."""
        _require_non_empty(self.model_id, "model_id")
        _require_non_empty(self.engine, "engine")
        _require_non_empty(self.revision, "revision")
        for alias in self.aliases:
            _require_non_empty(alias, "alias")

        identifiers = (self.model_id, *self.aliases)
        if len(set(identifiers)) != len(identifiers):
            message = "The model ID and aliases must be unique."
            raise ValueError(message)

    def accepts(self, model_id: str) -> bool:
        """Return whether a request identifier selects this deployment."""
        return model_id == self.model_id or model_id in self.aliases
