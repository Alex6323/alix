"""The gate-coverage guard fails on the shapes it exists to catch."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "check_gate_coverage", Path(__file__).with_name("check-gate-coverage.py")
)
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)


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
        self.assertEqual(0, gate.main(), "every unit is gated or named")


if __name__ == "__main__":
    unittest.main()
