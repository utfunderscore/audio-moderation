"""Tests for model-independent ASR values."""

from collections.abc import Callable
from dataclasses import FrozenInstanceError
from pathlib import Path

import pytest

from socialguard_models.core import (
    AudioInput,
    ModelCapabilities,
    ModelInfo,
    ResponseFormat,
    Transcript,
    TranscriptionOption,
    TranscriptionOptions,
)


def _set_attribute(instance: object, name: str, value: object) -> None:
    """Attempt a dynamic assignment without suppressing static type errors."""
    setattr(instance, name, value)


def _options_with_empty_language() -> TranscriptionOptions:
    """Construct an invalid empty language hint."""
    return TranscriptionOptions(language=" ")


def _options_with_empty_prompt() -> TranscriptionOptions:
    """Construct an invalid empty prompt hint."""
    return TranscriptionOptions(prompt="")


def test_capabilities_default_to_json_only() -> None:
    """The base contract supports JSON without optional model hints."""
    capabilities = ModelCapabilities()

    assert not capabilities.language
    assert not capabilities.prompt
    assert capabilities.response_formats == frozenset({ResponseFormat.JSON})


def test_capabilities_require_a_response_format() -> None:
    """A model cannot advertise an unusable response contract."""
    with pytest.raises(ValueError, match="at least one response format"):
        ModelCapabilities(response_formats=frozenset())


def test_options_report_unsupported_capabilities() -> None:
    """Requested model hints are checked without being silently ignored."""
    options = TranscriptionOptions(language="en", prompt="Product names")

    assert options.unsupported_by(ModelCapabilities()) == frozenset(
        {
            TranscriptionOption.LANGUAGE,
            TranscriptionOption.PROMPT,
        },
    )


def test_supported_options_are_not_reported() -> None:
    """Capabilities remove the corresponding options from the rejection set."""
    capabilities = ModelCapabilities(language=True, prompt=True)
    options = TranscriptionOptions(language="en", prompt="Product names")

    assert not options.unsupported_by(capabilities)


@pytest.mark.parametrize(
    ("field_name", "options"),
    [
        ("language", TranscriptionOptions(language=None)),
        ("prompt", TranscriptionOptions(prompt=None)),
    ],
)
def test_absent_options_are_valid(
    field_name: str,
    options: TranscriptionOptions,
) -> None:
    """Optional values may be omitted."""
    assert getattr(options, field_name) is None


@pytest.mark.parametrize(
    ("field_name", "factory"),
    [
        ("language", _options_with_empty_language),
        ("prompt", _options_with_empty_prompt),
    ],
)
def test_present_options_must_not_be_empty(
    field_name: str,
    factory: Callable[[], TranscriptionOptions],
) -> None:
    """Present request hints must carry a value."""
    with pytest.raises(ValueError, match=f"{field_name} must not be empty"):
        factory()


def test_audio_input_preserves_request_metadata() -> None:
    """Audio inputs describe a prepared file without reading it."""
    audio = AudioInput(
        path=Path("request.wav"),
        filename="speech.wav",
        content_type="audio/wav",
    )

    assert audio.path == Path("request.wav")
    assert audio.filename == "speech.wav"
    assert audio.content_type == "audio/wav"


def test_model_info_accepts_canonical_id_and_aliases() -> None:
    """A deployment recognizes only its configured model identifiers."""
    info = ModelInfo(
        model_id="faster-whisper-large-v3",
        engine="faster-whisper",
        revision="0123456789abcdef",
        capabilities=ModelCapabilities(language=True),
        aliases=("large-v3",),
    )

    assert info.accepts("faster-whisper-large-v3")
    assert info.accepts("large-v3")
    assert not info.accepts("unknown")


@pytest.mark.parametrize(
    "aliases",
    [
        ("large-v3", "large-v3"),
        ("faster-whisper-large-v3",),
    ],
)
def test_model_info_rejects_ambiguous_aliases(aliases: tuple[str, ...]) -> None:
    """Duplicate identifiers cannot produce ambiguous routing."""
    with pytest.raises(ValueError, match="must be unique"):
        ModelInfo(
            model_id="faster-whisper-large-v3",
            engine="faster-whisper",
            revision="0123456789abcdef",
            capabilities=ModelCapabilities(),
            aliases=aliases,
        )


def test_core_values_are_immutable() -> None:
    """Request and result values cannot change after construction."""
    transcript = Transcript(text="Hello")

    with pytest.raises(FrozenInstanceError):
        _set_attribute(transcript, "text", "Changed")
