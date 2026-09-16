"""Roblox policy-head validation and SocialGuard evidence-score reduction."""

import math
from collections.abc import Iterable, Sequence
from typing import cast

from socialguard_models.moderation.contracts import ModerationScores

CATEGORY_HEADS: dict[str, tuple[str, ...]] = {
    "sexual": ("ABUSE_TYPE_SEXUAL_CONTENT", "ABUSE_TYPE_DATING_AND_ROMANTIC_CONTENT"),
    "hate_or_discrimination": ("ABUSE_TYPE_DISCRIMINATORY",),
    "harassment_or_abuse": ("ABUSE_TYPE_HARASSMENT", "ABUSE_TYPE_PROFANITY"),
    "violence_or_threats": ("ABUSE_TYPE_ILLEGAL_AND_REGULATED_CONTENT",),
    "asking_for_pii": ("ABUSE_TYPE_PRIVACY_ASKING_FOR_PII",),
}
EXPECTED_LABELS = frozenset(
    head for heads in CATEGORY_HEADS.values() for head in heads
) | {"ABUSE_TYPE_DISRUPTIVE_AUDIO"}


def validate_labels(labels: object) -> tuple[str, ...]:
    if not isinstance(labels, (list, tuple)):
        raise ValueError("model labels must be a sequence")
    values = cast(Sequence[object], labels)
    if (
        len(values) != len(EXPECTED_LABELS)
        or not all(isinstance(label, str) for label in values)
        or frozenset(values) != EXPECTED_LABELS
    ):
        raise ValueError("model labels do not match the eight Roblox toxicity heads")
    return cast(tuple[str, ...], tuple(values))


def map_scores(labels: Sequence[str], probabilities: Sequence[object]) -> ModerationScores:
    ordered_labels = validate_labels(labels)
    if len(probabilities) != len(ordered_labels):
        raise ValueError("model probability shape does not match its labels")
    scores: dict[str, float] = {}
    for label, value in zip(ordered_labels, probabilities, strict=True):
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise ValueError("model probabilities must be numeric")
        score = float(value)
        if not math.isfinite(score) or not 0.0 <= score <= 1.0:
            raise ValueError("model probabilities must be finite and in [0, 1]")
        scores[label] = score
    return cast(ModerationScores, {
        category: max(scores[head] for head in heads)
        for category, heads in CATEGORY_HEADS.items()
    })


def aggregate_scores(windows: Iterable[ModerationScores]) -> ModerationScores:
    """Consume all windows; any failure propagates instead of returning partial scores."""
    result: dict[str, float] = {}
    for scores in windows:
        for category in CATEGORY_HEADS:
            value = cast(dict[str, float], scores)[category]
            result[category] = max(result.get(category, 0.0), value)
    if not result:
        raise ValueError("no audio windows were moderated")
    return cast(ModerationScores, result)
