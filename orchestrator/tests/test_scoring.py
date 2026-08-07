from __future__ import annotations

import unittest

from orchestrator.scoring import BranchScore, recommend


class ScoringTests(unittest.TestCase):
    def test_cross_tests_rank_but_do_not_disqualify_candidates(self) -> None:
        robust = BranchScore(
            agent="a",
            cross_tests_passed=4,
            cross_tests_total=4,
            mutants_missed=0,
            unresolved_defects=0,
            pedantic_warnings=2,
            diff_loc=900,
            check_ok=True,
        )
        small_but_wrong = BranchScore(
            agent="b",
            cross_tests_passed=3,
            cross_tests_total=4,
            mutants_missed=0,
            unresolved_defects=0,
            pedantic_warnings=0,
            diff_loc=100,
            check_ok=True,
        )

        self.assertEqual("a", recommend([robust, small_but_wrong]))
        self.assertTrue(small_but_wrong.eligible)
        self.assertEqual(250, small_but_wrong.cross_test_penalty)

    def test_an_exact_score_tie_is_left_to_the_human(self) -> None:
        first = BranchScore("a", 1, 1, 0, 0, 0, 100, True)
        second = BranchScore("b", 1, 1, 0, 0, 0, 100, True)

        self.assertIsNone(recommend([first, second]))

    def test_a_mutation_survivor_is_scored_without_making_the_candidate_ineligible(
        self,
    ) -> None:
        failed = BranchScore("a", 10, 10, 1, 0, 0, 10, True)
        passed = BranchScore("b", 2, 2, 0, 0, 5, 500, True)

        self.assertTrue(failed.eligible)
        self.assertEqual("b", recommend([failed, passed]))

    def test_a_symmetric_comparison_with_a_failed_check_never_auto_lands(
        self,
    ) -> None:
        broken = BranchScore(
            "a",
            10,
            10,
            0,
            0,
            0,
            10,
            check_ok=False,
        )
        sound = BranchScore("b", 2, 2, 0, 0, 5, 500, True)

        self.assertIsNone(recommend([broken, sound]))
        self.assertIn("check failed", broken.ineligible_reasons)

    def test_one_eligible_asymmetric_candidate_can_be_recommended(self) -> None:
        candidate = BranchScore("a", 1, 1, 0, 0, 0, 10, True)

        self.assertEqual("a", recommend([candidate]))

    def test_the_first_live_run_requires_a_human_instead_of_contradicting_evidence(
        self,
    ) -> None:
        a = BranchScore("a", 3, 3, 1, 0, 885, 426, True)
        b = BranchScore("b", 1, 4, 0, 0, 886, 277, True)

        self.assertEqual("a", recommend([a, b]))
        self.assertTrue(a.eligible)
        self.assertTrue(b.eligible)
        self.assertEqual(100, a.mutant_penalty)
        self.assertEqual(750, b.cross_test_penalty)

    def test_only_warnings_added_beyond_the_frozen_base_are_scored(self) -> None:
        inherited = BranchScore(
            "a",
            1,
            1,
            0,
            0,
            885,
            100,
            True,
            base_pedantic_warnings=885,
        )
        added = BranchScore(
            "b",
            1,
            1,
            0,
            0,
            886,
            100,
            True,
            base_pedantic_warnings=885,
        )

        self.assertEqual(0, inherited.pedantic_warnings_added)
        self.assertEqual(1, added.pedantic_warnings_added)
        self.assertLess(inherited.penalty, added.penalty)
