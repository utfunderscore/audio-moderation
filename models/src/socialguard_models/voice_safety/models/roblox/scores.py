"""Map Roblox's independent safety heads to the shared category scores."""

from collections.abc import Mapping
from math import isfinite

from socialguard_models.voice_safety.models import VoiceSafetyScores

CATEGORY_LABELS: dict[str, tuple[str, ...]] = {
    "sexual": ("sexual_content", "dating_and_romantic_content"),
    "hate_or_discrimination": ("discriminatory",),
    "harassment_or_abuse": ("harassment", "profanity"),
    "violence_or_threats": ("illegal_and_regulated_content",),
    "asking_for_pii": ("privacy_asking_for_pii",),
}


def normalize_scores(label_scores: Mapping[str, float]) -> VoiceSafetyScores:
    """Take each category's maximum, failing on missing or invalid source scores.

    Input keys are the upstream config's ABUSE_TYPE_* labels. Disruptive audio
    is deliberately unmapped because it is not part of the shared taxonomy.
    """
    scores: dict[str, float] = {}
    for category, labels in CATEGORY_LABELS.items():
        values: list[float] = []
        for label in labels:
            key = f"ABUSE_TYPE_{label.upper()}"
            value = label_scores[key]
            # Validate before max(): NaN or an invalid non-winning head must not
            # disappear behind a valid score from another subcategory.
            if not isfinite(value) or not 0 <= value <= 1:
                message = f"Invalid probability for {key}"
                raise ValueError(message)
            values.append(value)
        scores[category] = max(values)
    return VoiceSafetyScores.model_validate(scores)
