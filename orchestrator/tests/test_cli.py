from __future__ import annotations

import unittest
from pathlib import Path

from orchestrator.cli import parse_args


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
                "codex",
            ]
        )

        self.assertEqual("run", args.command)
        self.assertEqual("asymmetric", args.mode)
        self.assertEqual(Path("spec.md"), args.spec)
        self.assertEqual("codex", args.implementer)
        self.assertEqual(3, args.max_fix_rounds)

    def test_resume_and_report_take_one_exact_run_directory(self) -> None:
        resume = parse_args(["resume", "--run-dir", "/runs/one"])
        report = parse_args(["report", "--run-dir", "/runs/one"])

        self.assertEqual(Path("/runs/one"), resume.run_dir)
        self.assertEqual(Path("/runs/one"), report.run_dir)
