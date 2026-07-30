from __future__ import annotations

import unittest

from orchestrator.protocol import next_asymmetric_phase, next_symmetric_phase


class ProtocolTransitionTests(unittest.TestCase):
    def test_symmetric_reviews_every_bounded_round_and_skips_empty_fixes(self) -> None:
        self.assertEqual(
            "REVIEW_ROUND_1",
            next_symmetric_phase("IMPLEMENT", max_fix_rounds=2, has_defects=False),
        )
        self.assertEqual(
            "REVIEW_ROUND_2",
            next_symmetric_phase(
                "REVIEW_ROUND_1", max_fix_rounds=2, has_defects=False
            ),
        )
        self.assertEqual(
            "FIX_ROUND_2",
            next_symmetric_phase(
                "REVIEW_ROUND_2", max_fix_rounds=2, has_defects=True
            ),
        )
        self.assertEqual(
            "SCORE",
            next_symmetric_phase("FIX_ROUND_2", max_fix_rounds=2, has_defects=False),
        )

    def test_symmetric_fix_advances_to_the_next_review(self) -> None:
        self.assertEqual(
            "REVIEW_ROUND_2",
            next_symmetric_phase("FIX_ROUND_1", 2, has_defects=False),
        )

    def test_asymmetric_only_fixes_failed_property_runs(self) -> None:
        self.assertEqual(
            "RUN",
            next_asymmetric_phase(
                "IMPLEMENT_PROPERTIES", max_fix_rounds=2, suite_passed=False
            ),
        )
        self.assertEqual(
            "SCORE",
            next_asymmetric_phase("RUN", max_fix_rounds=2, suite_passed=True),
        )
        self.assertEqual(
            "FIX_ROUND_1",
            next_asymmetric_phase("RUN", max_fix_rounds=2, suite_passed=False),
        )
        self.assertEqual(
            "RUN_ROUND_1",
            next_asymmetric_phase(
                "FIX_ROUND_1", max_fix_rounds=2, suite_passed=False
            ),
        )
        self.assertEqual(
            "FIX_ROUND_2",
            next_asymmetric_phase(
                "RUN_ROUND_1", max_fix_rounds=2, suite_passed=False
            ),
        )
        self.assertEqual(
            "SCORE",
            next_asymmetric_phase(
                "RUN_ROUND_2", max_fix_rounds=2, suite_passed=False
            ),
        )
