"""Model-agnostic contracts for voice-safety classification workers."""

from enum import StrEnum
from typing import Annotated, ClassVar

import modal
from pydantic import BaseModel, ConfigDict, Field


class VoiceSafetyModel(StrEnum):
    """Voice-safety worker implementations."""

    ROBLOX = "roblox"


class VoiceSafetyJob(BaseModel):
    """One audio clip, fetched through an internally generated presigned URL."""

    download_url: str = Field(repr=False)


type Probability = Annotated[float, Field(ge=0, le=1, allow_inf_nan=False)]


class VoiceSafetyScores(BaseModel):
    """Required normalized categories; scores are not thresholded decisions."""

    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid")

    sexual: Probability
    hate_or_discrimination: Probability
    harassment_or_abuse: Probability
    violence_or_threats: Probability
    asking_for_pii: Probability


class VoiceSafetyResult(BaseModel):
    """Result returned by every voice-safety worker."""

    scores: VoiceSafetyScores
    language_probs: dict[str, Probability] = Field(default_factory=dict)


type VoiceSafetyWorker = modal.Function[
    [VoiceSafetyJob],
    VoiceSafetyResult,
    VoiceSafetyResult,
]
