"""Inputs for moderation; result contracts belong to the future implementation."""

from dataclasses import dataclass

from socialguard_models.contracts import AudioTask


@dataclass(frozen=True, slots=True)
class ModerationTask(AudioTask):
    """Moderate an audio file together with its transcription."""

    model: str
    transcription: str
