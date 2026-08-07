from __future__ import annotations

import unittest

from orchestrator.models import AgentState, Finding, PhaseHistory, RunState
from orchestrator.report import render_report
from orchestrator.scoring import BranchScore


class ReportTests(unittest.TestCase):
    def test_report_contains_research_evidence_and_recommendation(self) -> None:
        state = RunState(
            run_id="run",
            mode="symmetric",
            repo="/repo",
            run_dir="/runs/run",
            base="main",
            base_sha="abc",
            phase="COMPLETE",
            agents={
                "a": AgentState("/c", "agent/a/run", "c1"),
                "b": AgentState("/d", "agent/b/run", "d1"),
            },
            rounds_completed=1,
            max_fix_rounds=2,
            implementer="a",
            spec_hash="spec-sha",
            prompt_hashes={"implement": "prompt-sha"},
            findings=[
                Finding(
                    "F1",
                    "b",
                    "a",
                    "defect",
                    "findings/F1.patch",
                    True,
                    True,
                    "overflow repro",
                    test_name="overflow_is_rejected",
                    real_user_path="Import a supported deck containing an overflowing value.",
                    impact="The import process exits after partially writing the deck.",
                    observed="assertion failed: import must be atomic",
                )
            ],
            history=[
                PhaseHistory(
                    "IMPLEMENT",
                    "start",
                    "end",
                    True,
                    duration_seconds=2.5,
                    detail="human review: existing test lines changed",
                )
            ],
            token_usage={"a": 1000, "b": 800},
            costs_usd={"a": 0.25},
        )
        scores = [
            BranchScore("a", 2, 2, 0, 0, 0, 120, True),
            BranchScore("b", 1, 2, 1, 0, 1, 90, True),
        ]

        report = render_report(state, scores, divergence_notes=["different cache key"])

        self.assertIn("symmetric", report)
        self.assertIn("spec-sha", report)
        self.assertIn("overflow repro", report)
        self.assertIn("Import a supported deck", report)
        self.assertIn("partially writing", report)
        self.assertIn("2.50", report)
        self.assertIn("human review: existing test lines changed", report)
        self.assertIn("$0.2500", report)
        self.assertIn("a", report)
        self.assertIn("eligible", report)
        self.assertIn("1/2", report)
        self.assertIn("Check", report)
        self.assertIn("different cache key", report)

        blocked = render_report(
            state,
            [
                BranchScore(
                    "a",
                    3,
                    3,
                    1,
                    0,
                    885,
                    426,
                    check_ok=False,
                ),
                BranchScore("b", 1, 4, 0, 0, 886, 277, True),
            ],
        )
        self.assertIn(
            "Incomplete eligible comparison: human decision required.",
            blocked,
        )
