"""Tests for Roblox score normalization."""

import math
import unittest

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


class RobloxScoreTests(unittest.TestCase):
    def test_maps_upstream_heads_to_shared_categories(self) -> None:
        scores = normalize_scores(VALID_SCORES)

        self.assertEqual(
            scores.model_dump(),
            {
                "sexual": 0.6,
                "hate_or_discrimination": 0.2,
                "harassment_or_abuse": 0.7,
                "violence_or_threats": 0.5,
                "asking_for_pii": 0.1,
            },
        )

    def test_requires_every_mapped_head(self) -> None:
        scores = dict(VALID_SCORES)
        del scores["ABUSE_TYPE_HARASSMENT"]

        with self.assertRaises(KeyError):
            normalize_scores(scores)

    def test_rejects_invalid_probabilities_even_when_they_do_not_win_max(self) -> None:
        for value in (-0.1, 1.1, math.nan, math.inf):
            with self.subTest(value=value):
                scores = dict(VALID_SCORES)
                scores["ABUSE_TYPE_HARASSMENT"] = value
                with self.assertRaisesRegex(ValueError, "ABUSE_TYPE_HARASSMENT"):
                    normalize_scores(scores)


if __name__ == "__main__":
    unittest.main()
