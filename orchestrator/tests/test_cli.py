from __future__ import annotations

import argparse
import unittest
from pathlib import Path

from orchestrator.cli import _seat, parse_args


class CliTests(unittest.TestCase):
    def test_run_parses_both_protocol_controls(self) -> None:
        args = parse_args(
            [
                "run",
                "--mode",
                "asymmetric",
                "--spec",
                "spec.md",
                "--plan",
                "plan.md",
                "--repo",
                "/repo",
                "--base",
                "main",
                "--run-dir",
                "/runs",
                "--max-fix-rounds",
                "3",
                "--implementer",
                "b",
                "--agent-a",
                "claude:opus",
                "--agent-b",
                "claude:sonnet",
            ]
        )

        self.assertEqual("run", args.command)
        self.assertEqual("asymmetric", args.mode)
        self.assertEqual(Path("spec.md"), args.spec)
        self.assertEqual("b", args.implementer)
        self.assertEqual("claude:opus", args.agent_a)
        self.assertEqual("claude:sonnet", args.agent_b)
        self.assertEqual(3, args.max_fix_rounds)

    def test_a_seat_spec_splits_backend_from_model(self) -> None:
        self.assertEqual(("claude", "opus"), _seat("claude:opus", "--agent-a"))
        self.assertEqual(("codex", None), _seat("codex", "--agent-b"))

    def test_a_seat_spec_rejects_an_unknown_backend(self) -> None:
        with self.assertRaises(argparse.ArgumentTypeError):
            _seat("gemini:pro", "--agent-a")

    def test_resume_and_report_take_one_exact_run_directory(self) -> None:
        resume = parse_args(["resume", "--run-dir", "/runs/one"])
        report = parse_args(["report", "--run-dir", "/runs/one"])

        self.assertEqual(Path("/runs/one"), resume.run_dir)
        self.assertEqual(Path("/runs/one"), report.run_dir)
