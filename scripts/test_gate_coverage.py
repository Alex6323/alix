"""The gate-coverage guard fails on the shapes it exists to catch."""

from __future__ import annotations

import contextlib
import importlib.util
import io
import tempfile
import unittest
from pathlib import Path
from unittest import mock

SPEC = importlib.util.spec_from_file_location(
    "check_gate_coverage", Path(__file__).with_name("check-gate-coverage.py")
)
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)


def run_guard() -> int:
    """The guard's own report is noise inside a suite that expects it."""
    with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(
        io.StringIO()
    ):
        return gate.main()


class ParsesMakefileReachability(unittest.TestCase):
    def test_a_target_reached_through_two_hops_contributes_its_recipe(self):
        makefile = "check: lint\n\tfirst\n\nlint:\n\t$(MAKE) deep\n\ndeep:\n\tdeep-recipe\n"
        recipes = gate.reachable_recipes(gate.make_targets(makefile), ("check",))
        self.assertIn("deep-recipe", recipes, "a $(MAKE) hop is a reachable edge")

    def test_an_unreachable_target_contributes_nothing(self):
        makefile = "check: lint\n\tfirst\n\nlint:\n\tsecond\n\norphan:\n\torphan-recipe\n"
        recipes = gate.reachable_recipes(gate.make_targets(makefile), ("check",))
        self.assertNotIn(
            "orphan-recipe", recipes, "a target nothing calls must not count as gated"
        )

    def test_a_variable_assignment_is_not_a_target(self):
        makefile = "RUST := 1\ncheck: lint\n\tfirst\n\nlint:\n\tsecond\n"
        targets = gate.make_targets(makefile)
        self.assertNotIn("RUST", targets)


class ExpandsDirectoryChanges(unittest.TestCase):
    def test_a_cd_prefix_names_the_paths_its_command_reaches(self):
        line = "\tcd mobile/alix && flutter test test/"
        self.assertIn("mobile/alix/test/", gate.expand_cd(line))

    def test_a_line_without_cd_is_unchanged(self):
        line = "\tcargo test --manifest-path test-support/Cargo.toml"
        self.assertEqual(line, gate.expand_cd(line))


class GuardsTheExceptionList(unittest.TestCase):
    def test_every_existing_python_suite_is_enumerated(self):
        units = gate.units()
        self.assertIn(
            "orchestrator/pyproject.toml",
            units,
            "the standalone orchestrator suite must be classified by the gate",
        )
        self.assertIn(
            "scripts",
            units,
            "the repository tooling tests must stay reachable from a local gate",
        )

    def test_a_manifest_specific_test_does_not_cover_the_root_crate(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "test-support").mkdir()
            (root / ".github" / "workflows").mkdir(parents=True)
            (root / "Cargo.toml").write_text("[package]\nname='root'\n")
            (root / "test-support" / "Cargo.toml").write_text(
                "[package]\nname='support'\n"
            )
            (root / "Makefile").write_text(
                "check: test\n\ntest:\n\tcargo test --manifest-path test-support/Cargo.toml\n"
            )
            with mock.patch.multiple(
                gate,
                REPO_ROOT=root,
                MAKEFILE=root / "Makefile",
                WORKFLOWS=root / ".github" / "workflows",
                LOCAL_GATES=("check",),
                CI_ONLY={},
            ):
                self.assertEqual(
                    1,
                    run_guard(),
                    "a crate-specific command must not stand in for root cargo test",
                )

    def test_a_workflow_comment_does_not_prove_a_ci_only_suite_runs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "e2e" / "tests").mkdir(parents=True)
            workflows = root / ".github" / "workflows"
            workflows.mkdir(parents=True)
            (root / "Makefile").write_text("check:\n\t@true\n")
            (workflows / "ci.yml").write_text("# removed: make e2e\n")
            with mock.patch.multiple(
                gate,
                REPO_ROOT=root,
                MAKEFILE=root / "Makefile",
                WORKFLOWS=workflows,
                LOCAL_GATES=("check",),
                CI_ONLY={"e2e/tests": ("browser suite", "make e2e")},
            ):
                self.assertEqual(
                    1,
                    run_guard(),
                    "a marker in prose must not keep a stale CI exception alive",
                )

    def test_every_ci_only_entry_names_a_unit_that_exists(self):
        for unit in gate.CI_ONLY:
            self.assertIn(
                unit,
                gate.units(),
                f"{unit} is excused from local gating but is not in the tree",
            )

    def test_every_ci_only_marker_appears_in_a_workflow(self):
        workflows = gate.workflow_text()
        for unit, (_reason, marker) in gate.CI_ONLY.items():
            self.assertIn(
                marker,
                workflows,
                f"{unit} claims CI runs `{marker}`, and no workflow does",
            )

    def test_the_tree_passes_its_own_guard(self):
        self.assertEqual(0, run_guard(), "every unit is gated or named")


class ReadsRecipeCommands(unittest.TestCase):
    def test_a_manifest_run_is_not_a_whole_crate_test(self):
        rows = [
            ("cargo test", True),
            ("RUSTFLAGS=-Dwarnings cargo test", True),
            ("cargo nextest run", True),
            ("cargo test --locked --manifest-path test-support/Cargo.toml", False),
            ("cargo test --manifest-path=test-support/Cargo.toml", False),
            ("cargo build", False),
            ("cargo check --all-targets", False),
            ("flutter test", False),
        ]
        for line, expected in rows:
            with self.subTest(line=line):
                command = gate.commands(line)[0]
                self.assertEqual(
                    expected,
                    gate.is_whole_crate_test(command),
                    f"`{line}` was read as whole-crate={not expected}",
                )

    def test_a_path_counts_only_where_it_is_a_whole_argument(self):
        rows = [
            ("flutter test mobile/alix/test/", "mobile/alix/test", True),
            ("python3 -m unittest discover -s scripts", "scripts", True),
            ("python3 scripts/fmt-roadmap.py", "scripts", False),
            ("npm test e2e/unit-extra", "e2e/unit", False),
        ]
        for line, unit, expected in rows:
            with self.subTest(line=line, unit=unit):
                command = gate.commands(line)[0]
                self.assertEqual(
                    expected,
                    gate.names_path(command, unit),
                    f"`{line}` was read as naming={not expected} for {unit}",
                )

    def test_a_chained_command_is_split_at_the_operator(self):
        parsed = gate.commands("\tcd mobile/alix && flutter test test/")
        self.assertEqual(["cd", "mobile/alix"], parsed[0])
        self.assertEqual(["flutter", "test", "test/"], parsed[1])


class ReadsWorkflowExecution(unittest.TestCase):
    def test_only_executed_text_is_extracted(self):
        yaml = (
            "jobs:\n"
            "  gate:\n"
            "    name: make phantom-inline\n"
            "    steps:\n"
            "      # make phantom-comment\n"
            "      - run: make inline\n"
            "      - name: block\n"
            "        run: |\n"
            "          make block \\\n"
            "            SECONDS=600\n"
            "          # make phantom-in-block\n"
            "      - run: make after-block\n"
        )
        values = "\n".join(gate.run_values(yaml))
        for marker in ("make inline", "make block", "SECONDS=600", "make after-block"):
            self.assertIn(marker, values, f"{marker} is executed and must be extracted")
        for marker in ("phantom-inline", "phantom-comment", "phantom-in-block"):
            self.assertNotIn(marker, values, f"{marker} is prose and must not count")


class DiscoversUnitsFromEvidence(unittest.TestCase):
    def test_every_directory_holding_tests_is_a_unit_or_a_crate_subdirectory(self):
        _manifests, test_dirs = gate.walk()
        units = set(gate.units())
        crates = {
            str(Path(unit).parent.as_posix())
            for unit in units
            if Path(unit).name == "Cargo.toml"
        }
        for directory in sorted(test_dirs):
            with self.subTest(directory=directory):
                path = Path(directory)
                claimed = path.name in gate.CARGO_SUBDIRS and (
                    path.parent.as_posix() in crates
                )
                self.assertTrue(
                    directory in units or claimed,
                    f"{directory} holds tests that no unit accounts for",
                )

    def test_the_walk_does_not_descend_into_build_output(self):
        manifests, _test_dirs = gate.walk()
        for manifest in manifests:
            self.assertFalse(
                gate.SKIP_DIRS & set(Path(manifest).parts),
                f"{manifest} came from a directory the walk must prune",
            )


class SeparatesBlockingFromScheduled(unittest.TestCase):
    def test_each_workflow_is_classified_by_what_actually_triggers_it(self):
        rows = [
            ("ci.yml", True),
            ("fuzz-weekly.yml", False),
            ("mobile-release.yml", False),
            ("mutants-branch.yml", True),
        ]
        for name, expected in rows:
            with self.subTest(workflow=name):
                path = gate.WORKFLOWS / name
                self.assertEqual(
                    expected,
                    gate.gates_a_push(path.read_text()),
                    f"{name} was classified as push-gating={not expected}",
                )

    def test_a_scheduled_job_does_not_satisfy_a_push_responsibility(self):
        rows = [
            ("  schedule:\n    - cron: '0 0 * * 0'", 1),
            ("  push:\n    branches: [main]", 0),
        ]
        for triggers, expected in rows:
            with self.subTest(triggers=triggers.split(":")[0].strip()):
                with tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    (root / "e2e" / "tests").mkdir(parents=True)
                    (root / "e2e" / "tests" / "a.spec.ts").write_text("")
                    workflows = root / ".github" / "workflows"
                    workflows.mkdir(parents=True)
                    (root / "Makefile").write_text("check:\n\t@true\n")
                    (workflows / "ci.yml").write_text(
                        f"on:\n{triggers}\njobs:\n  a:\n    steps:\n      - run: make e2e\n"
                    )
                    with mock.patch.multiple(
                        gate,
                        REPO_ROOT=root,
                        MAKEFILE=root / "Makefile",
                        WORKFLOWS=workflows,
                        LOCAL_GATES=("check",),
                        CI_ONLY={"e2e/tests": ("browser suite", "make e2e")},
                        SCHEDULED=set(),
                    ):
                        self.assertEqual(
                            expected,
                            run_guard(),
                            "only a push-gating workflow proves a per-push suite runs",
                        )


if __name__ == "__main__":
    unittest.main()
