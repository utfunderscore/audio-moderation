"""Tests for Roblox score normalization."""

import math

import pytest

from socialguard_models.voice_safety.models.roblox.scores import normalize_scores

VALID_SCORES = {
    "ABUSE_TYPE_PRIVACY_ASKING_FOR_PII": 0.1,
    "ABUSE_TYPE_DISCRIMINATORY": 0.2,
    "ABUSE_TYPE_HARASSMENT": 0.3,
    "ABUSE_TYPE_SEXUAL_CONTENT": 0.4,
    "ABUSE_TYPE_ILLEGAL_AND_REGULATED_CONTENT": 0.5,
    "ABUSE_TYPE_DATING_AND_ROMANTIC_CONTENT": 0.6,
    "ABUSE_TYPE_PROFANITY": 0.7,
    "ABUSE_TYPE_DISRUPTIVE_AUDIO": 0.8,
}


def test_maps_upstream_heads_to_shared_categories() -> None:
    """The shared taxonomy keeps each mapped upstream maximum."""
    scores = normalize_scores(VALID_SCORES)

    assert scores.model_dump() == {
        "sexual": 0.6,
        "hate_or_discrimination": 0.2,
        "harassment_or_abuse": 0.7,
        "violence_or_threats": 0.5,
        "asking_for_pii": 0.1,
    }


def test_requires_every_mapped_head() -> None:
    """Missing mapped heads are rejected rather than silently defaulted."""
    scores = dict(VALID_SCORES)
    del scores["ABUSE_TYPE_HARASSMENT"]

    with pytest.raises(KeyError):
        normalize_scores(scores)


def test_rejects_invalid_probabilities_even_when_they_do_not_win_max() -> None:
    """Every source head must be a finite probability before normalization."""
    for value in [-0.1, 1.1, math.nan, math.inf]:
        scores = dict(VALID_SCORES)
        scores["ABUSE_TYPE_HARASSMENT"] = value

        with pytest.raises(ValueError, match="ABUSE_TYPE_HARASSMENT"):
            normalize_scores(scores)
