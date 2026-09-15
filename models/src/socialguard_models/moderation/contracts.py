"""Moderation inputs and category-score output contracts."""

from dataclasses import dataclass
from typing import TypedDict

from socialguard_models.contracts import AudioTask, FailedOutcome


@dataclass(frozen=True, slots=True)
class ModerationTask(AudioTask):
    """Moderate an audio file together with its transcription."""

    model: str
    transcription: str


class ModerationScores(TypedDict):
    """The five required category scores returned by a moderation model.

    Values are floats; their range and interpretation are model-defined.
    """

    sexual: float
    hate_or_discrimination: float
    harassment_or_abuse: float
    violence_or_threats: float
    asking_for_pii: float


@dataclass(frozen=True, slots=True)
class CompletedOutcome:
    """A successful moderation result, parallel to a completed transcription."""

    scores: ModerationScores


type ModerationOutcome = CompletedOutcome | FailedOutcome
