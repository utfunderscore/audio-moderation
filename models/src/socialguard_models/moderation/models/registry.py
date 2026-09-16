"""Moderation GPU submission functions, registered in source at deployment time."""

from collections.abc import Awaitable, Callable

from socialguard_models.model_job import SubmittedModel
from socialguard_models.moderation.contracts import ModerationScores
from socialguard_models.moderation.models.roblox_voice_safety import (
    PUBLIC_MODEL_ID,
    submit_roblox_voice_safety,
)

type ModelSubmitter = Callable[
    [str, str], Awaitable[SubmittedModel[ModerationScores]]
]

# Each entry submits (audio_url, transcription) using a model's spawn.aio method
# and returns the call handle without waiting for inference. Import concrete
# submitters here so HTTP and CPU containers load the same registry.
MODEL_SUBMITTERS: dict[str, ModelSubmitter] = {
    PUBLIC_MODEL_ID: submit_roblox_voice_safety,
}
