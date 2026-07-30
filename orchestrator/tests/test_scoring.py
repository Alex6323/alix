from __future__ import annotations

import unittest

from orchestrator.scoring import BranchScore, recommend


def score(agent: str, **overrides: object) -> BranchScore:
    fields: dict[str, object] = {
        "agent": agent,
        "cross_tests_passed": 1,
        "cross_tests_total": 1,
        "mutants_missed": 0,
        "unresolved_defects": 0,
        "pedantic_warnings": 0,
        "diff_loc": 100,
        "check_ok": True,
    }
    fields.update(overrides)
    return BranchScore(**fields)  # type: ignore[arg-type]


class ScoringTests(unittest.TestCase):
    def test_verified_defects_and_cross_tests_dominate_size(self) -> None:
        robust = score(
            "claude",
            cross_tests_passed=4,
            cross_tests_total=4,
            mutants_missed=1,
            pedantic_warnings=2,
            diff_loc=900,
        )
        small_but_wrong = score(
            "codex",
            cross_tests_passed=3,
            cross_tests_total=4,
            unresolved_defects=1,
            diff_loc=100,
        )

        self.assertEqual("claude", recommend([robust, small_but_wrong]))

    def test_an_exact_score_tie_is_left_to_the_human(self) -> None:
        self.assertIsNone(recommend([score("claude"), score("codex")]))

    def test_a_failed_check_cannot_win(self) -> None:
        failed = score("claude", cross_tests_total=10, cross_tests_passed=10, check_ok=False)
        passed = score(
            "codex",
            cross_tests_passed=1,
            cross_tests_total=2,
            mutants_missed=5,
            pedantic_warnings=5,
            diff_loc=500,
        )

        self.assertEqual("codex", recommend([failed, passed]))

    def test_no_eligible_branch_recommends_nothing(self) -> None:
        self.assertIsNone(
            recommend([score("claude", check_ok=False), score("codex", check_ok=False)])
        )

    def test_a_missed_mutant_is_graded_not_disqualifying(self) -> None:
        # The real run's shape: one surviving mutant against a branch that
        # fails three of the opponent's regression tests.
        thorough = score(
            "claude",
            cross_tests_passed=3,
            cross_tests_total=3,
            mutants_missed=1,
            pedantic_warnings=885,
            diff_loc=426,
        )
        leaky = score(
            "codex",
            cross_tests_passed=1,
            cross_tests_total=4,
            pedantic_warnings=886,
            diff_loc=277,
        )

        self.assertEqual("claude", recommend([thorough, leaky]))

    def test_a_defect_costs_the_same_whoever_filed_it(self) -> None:
        # A finding filed against you and a defect only the opponent's test
        # catches are the same thing: one defect still on your branch.
        filed_against_me = score("claude", unresolved_defects=1)
        caught_by_their_test = score(
            "codex",
            cross_tests_passed=0,
            cross_tests_total=1,
        )

        self.assertEqual(
            filed_against_me.penalty,
            caught_by_their_test.penalty,
            "a defect's cost must not depend on which agent found it",
        )

    def test_cross_tests_failed_counts_every_failure(self) -> None:
        for passed, total, expected in [(3, 3, 0), (1, 4, 3), (0, 2, 2), (5, 3, 0)]:
            with self.subTest(passed=passed, total=total):
                branch = score(
                    "claude",
                    cross_tests_passed=passed,
                    cross_tests_total=total,
                )
                self.assertEqual(expected, branch.cross_tests_failed)
