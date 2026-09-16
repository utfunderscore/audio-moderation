import math

import pytest

from socialguard_models.moderation.models.roblox_voice_safety_scores import (
    EXPECTED_LABELS,
    aggregate_scores,
    map_scores,
    validate_labels,
)


def test_mapping_uses_labels_and_maximum_without_normalizing() -> None:
    source = {
        "ABUSE_TYPE_PRIVACY_ASKING_FOR_PII": 0.11,
        "ABUSE_TYPE_DISCRIMINATORY": 0.22,
        "ABUSE_TYPE_HARASSMENT": 0.33,
        "ABUSE_TYPE_SEXUAL_CONTENT": 0.44,
        "ABUSE_TYPE_ILLEGAL_AND_REGULATED_CONTENT": 0.55,
        "ABUSE_TYPE_DATING_AND_ROMANTIC_CONTENT": 0.66,
        "ABUSE_TYPE_PROFANITY": 0.77,
        "ABUSE_TYPE_DISRUPTIVE_AUDIO": 0.99,
    }
    expected = {
        "sexual": 0.66,
        "hate_or_discrimination": 0.22,
        "harassment_or_abuse": 0.77,
        "violence_or_threats": 0.55,
        "asking_for_pii": 0.11,
    }
    for labels in (list(source), list(reversed(source))):
        assert map_scores(labels, [source[label] for label in labels]) == expected
    source["ABUSE_TYPE_HARASSMENT"] = 0.88
    source["ABUSE_TYPE_SEXUAL_CONTENT"] = 0.99
    result = map_scores(list(source), list(source.values()))
    assert result["harassment_or_abuse"] == 0.88
    assert result["sexual"] == 0.99


@pytest.mark.parametrize("labels", [None, "labels", [], list(EXPECTED_LABELS)[:-1], ["duplicate"] * 8, [[]] * 8])
def test_rejects_invalid_model_labels(labels: object) -> None:
    with pytest.raises(ValueError):
        validate_labels(labels)


@pytest.mark.parametrize("value", [math.nan, math.inf, -math.inf, -0.1, 1.1, True, "0.5", None, [0.5]])
def test_rejects_invalid_probability_even_for_ignored_head(value: object) -> None:
    labels = sorted(EXPECTED_LABELS)
    values = [0.1] * 8
    values[labels.index("ABUSE_TYPE_DISRUPTIVE_AUDIO")] = value
    with pytest.raises(ValueError):
        map_scores(labels, values)


def test_rejects_wrong_output_length() -> None:
    with pytest.raises(ValueError, match="shape"):
        map_scores(sorted(EXPECTED_LABELS), [0.1] * 7)


def test_late_violation_survives_recording_aggregation() -> None:
    labels = sorted(EXPECTED_LABELS)
    benign = map_scores(labels, [0.01] * 8)
    late_values = [0.0] * 8
    late_values[labels.index("ABUSE_TYPE_PRIVACY_ASKING_FOR_PII")] = 0.97
    late = map_scores(labels, late_values)
    result = aggregate_scores([benign] * 24 + [late])
    assert result == {**benign, "asking_for_pii": 0.97}
    assert all(type(value) is float for value in result.values())


def test_failed_window_does_not_return_partial_success() -> None:
    def windows():
        yield map_scores(sorted(EXPECTED_LABELS), [0.1] * 8)
        raise RuntimeError("window failed")

    with pytest.raises(RuntimeError, match="window failed"):
        aggregate_scores(windows())
    with pytest.raises(ValueError, match="no audio windows"):
        aggregate_scores([])
