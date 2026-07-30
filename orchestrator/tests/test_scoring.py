from __future__ import annotations

import unittest

from orchestrator.scoring import BranchScore, recommend


class ScoringTests(unittest.TestCase):
    def test_verified_defects_and_cross_tests_dominate_size(self) -> None:
        robust = BranchScore(
            agent="claude",
            cross_tests_passed=4,
            cross_tests_total=4,
            mutants_missed=1,
            unresolved_defects=0,
            pedantic_warnings=2,
            diff_loc=900,
            gate_ok=True,
        )
        small_but_wrong = BranchScore(
            agent="codex",
            cross_tests_passed=3,
            cross_tests_total=4,
            mutants_missed=0,
            unresolved_defects=1,
            pedantic_warnings=0,
            diff_loc=100,
            gate_ok=True,
        )

        self.assertEqual("claude", recommend([robust, small_but_wrong]))

    def test_an_exact_score_tie_is_left_to_the_human(self) -> None:
        first = BranchScore("claude", 1, 1, 0, 0, 0, 100, True)
        second = BranchScore("codex", 1, 1, 0, 0, 0, 100, True)

        self.assertIsNone(recommend([first, second]))

    def test_a_failed_gate_cannot_win(self) -> None:
        failed = BranchScore("claude", 10, 10, 0, 0, 0, 10, False)
        passed = BranchScore("codex", 1, 2, 5, 0, 5, 500, True)

        self.assertEqual("codex", recommend([failed, passed]))
