import contextlib
import io
import json
import os
import pathlib
import subprocess
import tempfile
import unittest
from unittest import mock

from mutants_timeout_retry import (
    FocusedRun,
    RetryRecord,
    focused_command,
    main,
    render_summary,
    run_focused_mutant,
    run_timeout_retries,
)


def mutant_outcome(
    name: str,
    summary: str,
    build_seconds: float,
    test_seconds: float,
) -> dict:
    test_status: str | dict[str, int]
    if summary == "Timeout":
        test_status = "Timeout"
    elif summary == "CaughtMutant":
        test_status = {"Failure": 101}
    else:
        test_status = "Success"
    return {
        "scenario": {"Mutant": {"name": name}},
        "summary": summary,
        "phase_results": [
            {
                "phase": "Build",
                "duration": build_seconds,
                "process_status": "Success",
            },
            {
                "phase": "Test",
                "duration": test_seconds,
                "process_status": test_status,
            },
        ],
    }


def write_outcomes(path: pathlib.Path, outcomes: list[dict]) -> None:
    path.mkdir(parents=True, exist_ok=True)
    (path / "outcomes.json").write_text(
        json.dumps({"outcomes": outcomes}),
        encoding="utf-8",
    )


def write_focused_outcomes(path: pathlib.Path, outcomes: list[dict]) -> None:
    """Mirror cargo-mutants' CARGO_MUTANTS_OUTPUT=<path> layout."""
    write_outcomes(path / "mutants.out", outcomes)


class MutantsTimeoutRetryTests(unittest.TestCase):
    def setUp(self):
        self.tempdir = tempfile.TemporaryDirectory()
        self.out = pathlib.Path(self.tempdir.name) / "mutants.out"
        self.out.mkdir()

    def tearDown(self):
        self.tempdir.cleanup()

    def plant_initial_timeout(
        self,
        name: str,
        build_seconds: float = 2.0,
        test_seconds: float = 40.0,
    ) -> None:
        (self.out / "timeout.txt").write_text(name + "\n", encoding="utf-8")
        for filename in ("caught.txt", "missed.txt", "unviable.txt"):
            (self.out / filename).write_text("", encoding="utf-8")
        write_outcomes(
            self.out,
            [mutant_outcome(name, "Timeout", build_seconds, test_seconds)],
        )

    def test_a_planted_timeout_retries_once_and_records_both_runtimes(self):
        name = "src/store.rs:410:13: replace match guard with true"
        self.plant_initial_timeout(name)
        calls = []

        def caught_runner(mutant, timeout_seconds, retry_out):
            calls.append((mutant, timeout_seconds))
            write_focused_outcomes(
                retry_out,
                [mutant_outcome(mutant, "CaughtMutant", 3.0, 12.25)],
            )
            return 0

        records = run_timeout_retries(self.out, caught_runner)

        self.assertEqual(
            [(name, 80)], calls, "one retry uses twice the 40s timeout"
        )
        self.assertEqual(
            name + "\n",
            (self.out / "timeout-initial.txt").read_text(),
            "the initial timeout remains auditable",
        )
        self.assertEqual("", (self.out / "timeout.txt").read_text(), "caught closes")
        self.assertEqual(
            name + "\n",
            (self.out / "caught.txt").read_text(),
            "focused catch is reclassified",
        )
        self.assertEqual(
            42.0,
            records[0].original_runtime_seconds,
            "original runtime sums build and timed-out test phases",
        )
        self.assertEqual(
            15.25,
            records[0].focused_runtime_seconds,
            "focused runtime sums its build and test phases",
        )
        summary = render_summary(records)
        self.assertIn("42.000s", summary, "summary includes original runtime")
        self.assertIn("15.250s", summary, "summary includes focused runtime")
        self.assertIn("caught", summary, "summary includes focused outcome")

    def test_a_mutant_that_times_out_focused_remains_an_open_timeout(self):
        name = "src/deck.rs:635:22: replace && with || in edge_satisfied"
        self.plant_initial_timeout(name, build_seconds=1.0, test_seconds=20.0)

        def timeout_runner(mutant, timeout_seconds, retry_out):
            write_focused_outcomes(
                retry_out,
                [mutant_outcome(mutant, "Timeout", 2.0, 40.0)],
            )
            return 3

        records = run_timeout_retries(self.out, timeout_runner)

        self.assertEqual(
            name + "\n",
            (self.out / "timeout.txt").read_text(),
            "a repeated timeout stays in the open timeout list",
        )
        self.assertEqual(
            "", (self.out / "caught.txt").read_text(), "timeout is not a catch"
        )
        self.assertTrue(records[0].open, "a focused timeout remains open")
        self.assertEqual(
            "timeout", records[0].focused_outcome, "focused outcome stays explicit"
        )

    def test_a_retry_that_only_finishes_under_the_longer_budget_stays_visible(self):
        name = "src/store.rs:1618:13: replace && with || in sync_conflicts"
        self.plant_initial_timeout(name, build_seconds=1.0, test_seconds=20.0)

        def missed_runner(mutant, timeout_seconds, retry_out):
            write_focused_outcomes(
                retry_out,
                [mutant_outcome(mutant, "MissedMutant", 2.0, 31.0)],
            )
            return 2

        records = run_timeout_retries(self.out, missed_runner)

        self.assertEqual(
            "",
            (self.out / "timeout.txt").read_text(),
            "a completed retry is not a timeout",
        )
        self.assertEqual(
            name + "\n",
            (self.out / "missed.txt").read_text(),
            "longer-budget survivor becomes a miss",
        )
        self.assertTrue(records[0].open, "a focused miss remains open")
        self.assertEqual(
            "missed", records[0].focused_outcome, "focused outcome stays explicit"
        )
        self.assertIn(
            r"replace && with \|\|",
            render_summary(records),
            "summary escapes table punctuation in the mutant name",
        )
        report = json.loads((self.out / "timeout-retries.json").read_text())
        self.assertEqual(
            name,
            report["retries"][0]["mutant"],
            "machine report retains the longer-budget survivor",
        )

    def test_the_focused_command_selects_exactly_one_escaped_mutant(self):
        name = "src/a+b.rs:7:9: replace (x) with x * 2"

        command = focused_command(name, 41)

        self.assertEqual(
            [
                "cargo",
                "mutants",
                "--baseline",
                "skip",
                "--jobs",
                "1",
                "--timeout",
                "41",
                "--re",
                r"^(?:src/a\+b\.rs:7:9:\ replace\ \(x\)\ with\ x\ \*\ 2)$",
            ],
            command,
            "focused command is anchored, escaped, serial, and skips the baseline",
        )

    def test_any_selection_other_than_the_exact_mutant_never_starts_a_run(self):
        root = pathlib.Path(self.tempdir.name) / "repo"
        source = root / "src/lib.rs"
        source.parent.mkdir(parents=True)
        source.write_text("fn before() {}\nfn target() {}\n", encoding="utf-8")
        name = "src/lib.rs:2:4: replace target -> bool with true"
        listed_cases = [
            [],
            ["another mutant"],
            [name, "another mutant"],
            [name, name],
        ]

        for listed in listed_cases:
            calls = []

            def command_runner(command, **kwargs):
                calls.append(command)
                output = "".join(f"{candidate}\n" for candidate in listed)
                return subprocess.CompletedProcess(command, 0, output, "")

            with self.subTest(listed=listed), mock.patch(
                "mutants_timeout_retry.subprocess.run",
                side_effect=command_runner,
            ):
                result = run_focused_mutant(name, 41, self.out / "retry", root)

            self.assertEqual(
                1,
                len(calls),
                f"selection {listed!r} stops after one list command",
            )
            self.assertIn("--list", calls[0], f"selection {listed!r} is checked")
            self.assertNotEqual(
                "",
                result.detail,
                f"selection {listed!r} records why it was refused",
            )

    def test_ci_forced_color_cannot_change_the_exact_list_selection(self):
        root = pathlib.Path(self.tempdir.name) / "repo"
        source = root / "src/lib.rs"
        source.parent.mkdir(parents=True)
        source.write_text("fn before() {}\nfn target() {}\n", encoding="utf-8")
        name = "src/lib.rs:2:4: replace target -> bool with true"
        calls = []

        def command_runner(command, **kwargs):
            calls.append((command, kwargs["env"]))
            output = ""
            if "--list" in command:
                color = (
                    command[command.index("--colors") + 1]
                    if "--colors" in command
                    else kwargs["env"].get("CARGO_TERM_COLOR", "auto")
                )
                output = (
                    f"\x1b[38;5;13m{name}\x1b[0m\n"
                    if color == "always"
                    else name + "\n"
                )
            return subprocess.CompletedProcess(command, 0, output, "")

        with (
            mock.patch.dict(os.environ, {"CARGO_TERM_COLOR": "always"}),
            mock.patch(
                "mutants_timeout_retry.subprocess.run",
                side_effect=command_runner,
            ),
        ):
            result = run_focused_mutant(name, 41, self.out / "retry", root)

        self.assertEqual(2, len(calls), "an exact colored selection starts the run")
        list_command, list_environment = calls[0]
        self.assertEqual(
            "always",
            list_environment["CARGO_TERM_COLOR"],
            "the test reproduces the CI environment",
        )
        self.assertEqual(
            "never",
            list_command[list_command.index("--colors") + 1],
            "the machine-parsed list explicitly disables color",
        )
        self.assertEqual(0, result.exit_code, "the focused run starts successfully")
        self.assertEqual("", result.detail, "ANSI styling cannot cause a refusal")

    def test_a_selection_refusal_stays_open_with_its_detail(self):
        name = "src/store.rs:410:13: replace match guard with true"
        self.plant_initial_timeout(name)

        records = run_timeout_retries(
            self.out,
            lambda mutant, timeout_seconds, retry_out: FocusedRun(
                1,
                "focused selection expected one exact mutant, listed 2",
            ),
        )

        self.assertEqual(
            name + "\n",
            (self.out / "timeout.txt").read_text(encoding="utf-8"),
            "a refused selection remains an open timeout",
        )
        self.assertTrue(records[0].open, "a refused selection remains open")
        self.assertEqual(
            "error", records[0].focused_outcome, "refusal is an explicit error"
        )
        self.assertIn(
            "listed 2",
            records[0].detail,
            "the refusal detail reaches the retry record",
        )
        report = json.loads(
            (self.out / "timeout-retries.json").read_text(encoding="utf-8")
        )
        self.assertIn(
            "listed 2",
            report["retries"][0]["detail"],
            "the refusal detail reaches the machine report",
        )

    def test_one_exact_listed_mutant_runs_with_the_same_line_filter(self):
        root = pathlib.Path(self.tempdir.name) / "repo"
        source = root / "src/lib.rs"
        source.parent.mkdir(parents=True)
        source.write_text("fn before() {}\nfn target() {}\n", encoding="utf-8")
        name = "src/lib.rs:2:4: replace target -> bool with true"
        calls = []

        def command_runner(command, **kwargs):
            calls.append(command)
            output = name + "\n" if "--list" in command else ""
            return subprocess.CompletedProcess(command, 0, output, "")

        with mock.patch(
            "mutants_timeout_retry.subprocess.run",
            side_effect=command_runner,
        ):
            result = run_focused_mutant(name, 41, self.out / "retry", root)

        self.assertEqual(
            2, len(calls), "one exact selection starts one mutation run"
        )
        self.assertIn("--list", calls[0], "selection is listed before mutation")
        self.assertNotIn("--list", calls[1], "the second command runs the mutation")
        for index, command in enumerate(calls):
            self.assertIn("--in-diff", command, f"command {index} uses line filter")
            self.assertEqual(
                command[command.index("--in-diff") + 1],
                calls[0][calls[0].index("--in-diff") + 1],
                f"command {index} uses the same selection diff",
            )
        selection_path = pathlib.Path(
            calls[0][calls[0].index("--in-diff") + 1]
        )
        self.assertEqual(
            "diff --git a/src/lib.rs b/src/lib.rs\n"
            "--- a/src/lib.rs\n"
            "+++ b/src/lib.rs\n"
            "@@ -2 +2 @@\n"
            "-fn target() {}\n"
            "+fn target() {}\n",
            selection_path.read_text(encoding="utf-8"),
            "selection diff admits only the mutant's exact source line",
        )
        self.assertEqual(
            0, result.exit_code, "the exact mutation run exit is returned"
        )
        self.assertEqual("", result.detail, "an exact selection has no refusal detail")

    def test_the_cli_appends_every_retry_row_to_the_github_summary(self):
        summary_path = pathlib.Path(self.tempdir.name) / "summary.md"
        records = [
            RetryRecord(
                mutant="src/store.rs:410:13: replace match guard with true",
                original_runtime_seconds=42.0,
                focused_runtime_seconds=15.25,
                focused_timeout_seconds=80,
                focused_outcome="caught",
                open=False,
                runner_exit=0,
            )
        ]

        with (
            mock.patch.dict(os.environ, {"GITHUB_STEP_SUMMARY": str(summary_path)}),
            mock.patch(
                "mutants_timeout_retry.run_timeout_retries",
                return_value=records,
            ),
            contextlib.redirect_stdout(io.StringIO()),
        ):
            status = main([str(self.out)])

        self.assertEqual(0, status, "a complete report exits successfully")
        summary = summary_path.read_text(encoding="utf-8")
        self.assertIn("42.000s", summary, "GitHub summary includes original runtime")
        self.assertIn("15.250s", summary, "GitHub summary includes focused runtime")


if __name__ == "__main__":
    unittest.main()
