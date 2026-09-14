import copy
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parent.parent
GUARD = ROOT / "scripts" / "check-risk-matrix.py"


def complete_matrix():
    return {
        "schema": 1,
        "risk_classes": [
            {
                "id": "pure-domain-error",
                "critical_paths": ["identity", "scheduling"],
                "primary_evidence": [
                    {
                        "kind": "unit-property",
                        "command": "make test",
                        "cadence": "per-change",
                    }
                ],
                "state": "covered",
            },
            {
                "id": "parser-hostility",
                "critical_paths": ["parser", "locator"],
                "primary_evidence": [
                    {
                        "kind": "parser-unit-law",
                        "command": "make parser-hostility-test",
                        "cadence": "per-change",
                    },
                    {
                        "kind": "locator-unit-law",
                        "command": "make locator-hostility-test",
                        "cadence": "per-change",
                    }
                ],
                "state": "covered",
            },
            {
                "id": "data-loss",
                "critical_paths": ["zip-receive", "store", "os-matrix"],
                "primary_evidence": [
                    {
                        "kind": "integration-regression",
                        "command": "make test",
                        "cadence": "per-change",
                    },
                    {
                        "kind": "cross-compile",
                        "command": "make windows-check",
                        "cadence": "per-change",
                    },
                ],
                "state": "gap",
                "gap": "ZIP and store lack planted faults; OS jobs lack target reporting",
                "roadmap_anchor": "risk-class-test-matrix",
            },
            {
                "id": "api-drift",
                "critical_paths": ["api-contract"],
                "primary_evidence": [
                    {
                        "kind": "contract-integration",
                        "command": "make test",
                        "cadence": "per-change",
                    }
                ],
                "state": "covered",
            },
            {
                "id": "ui-control-no-op",
                "critical_paths": ["web-mobile-controls"],
                "primary_evidence": [
                    {
                        "kind": "browser-integration",
                        "command": "make e2e",
                        "cadence": "per-push",
                    },
                    {
                        "kind": "flutter-integration",
                        "command": "make mobile-test",
                        "cadence": "per-push",
                    },
                ],
                "state": "covered",
            },
            {
                "id": "concurrency-race",
                "critical_paths": ["soak"],
                "primary_evidence": [
                    {
                        "kind": "deterministic-regression",
                        "command": "make test",
                        "cadence": "per-change",
                    }
                ],
                "state": "gap",
                "gap": "no repeatable long-running concurrency workload",
                "roadmap_anchor": "risk-class-test-matrix",
            },
            {
                "id": "backend-cli-drift",
                "critical_paths": ["backend-cli"],
                "primary_evidence": [
                    {
                        "kind": "backend-matrix",
                        "command": "make check-backends",
                        "cadence": "manual",
                    }
                ],
                "state": "covered",
            },
            {
                "id": "ai-judgment",
                "critical_paths": ["grader-calibration"],
                "primary_evidence": [
                    {
                        "kind": "labeled-real-model",
                        "command": "make calibrate",
                        "cadence": "release",
                    }
                ],
                "state": "covered",
            },
            {
                "id": "release-defect",
                "critical_paths": ["packaged-artifact"],
                "primary_evidence": [
                    {
                        "kind": "package-smoke",
                        "command": "make package-verify",
                        "cadence": "release",
                    }
                ],
                "state": "covered",
            },
        ],
    }


class RiskMatrixGuardTests(unittest.TestCase):
    def run_guard(self, matrix, makefile=None, existing_paths=()):
        return self.run_guard_text(json.dumps(matrix), makefile, existing_paths)

    def run_guard_text(self, text, makefile=None, existing_paths=()):
        with tempfile.TemporaryDirectory() as raw:
            path = pathlib.Path(raw) / "risk-matrix.json"
            path.write_text(text, encoding="utf-8")
            for existing in existing_paths:
                (path.parent / existing).mkdir()
            command = [sys.executable, str(GUARD), "--matrix", str(path)]
            if makefile is not None:
                makefile_path = pathlib.Path(raw) / "Makefile"
                makefile_path.write_text(makefile, encoding="utf-8")
                command.extend(["--makefile", str(makefile_path)])
            return subprocess.run(
                command,
                cwd=ROOT,
                text=True,
                capture_output=True,
                check=False,
            )

    def assert_invalid(self, matrix, message, makefile=None, existing_paths=()):
        result = self.run_guard(matrix, makefile, existing_paths)
        self.assertEqual(1, result.returncode, result.stdout + result.stderr)
        self.assertIn(f"risk-matrix: {message}", result.stderr)

    def fixture_makefile(self, matrix):
        targets = {
            evidence["command"].split()[1]
            for row in matrix["risk_classes"]
            for evidence in row["primary_evidence"]
        }
        return "\n".join(f"{target}:\n\t@true\n" for target in sorted(targets))

    def test_the_complete_matrix_passes(self):
        matrix = complete_matrix()
        result = self.run_guard(matrix, self.fixture_makefile(matrix))

        self.assertEqual(0, result.returncode, result.stdout + result.stderr)
        self.assertIn("risk-matrix: 9 risk classes, 13 critical paths", result.stdout)

    def test_each_required_failure_class_fails_closed_when_missing(self):
        matrix = complete_matrix()
        for row in matrix["risk_classes"]:
            with self.subTest(risk_class=row["id"]):
                planted = copy.deepcopy(matrix)
                planted["risk_classes"] = [
                    candidate
                    for candidate in planted["risk_classes"]
                    if candidate["id"] != row["id"]
                ]
                self.assert_invalid(planted, f"missing risk class: {row['id']}")

    def test_each_required_critical_path_fails_closed_when_missing(self):
        matrix = complete_matrix()
        for row in matrix["risk_classes"]:
            for path in row["critical_paths"]:
                with self.subTest(critical_path=path):
                    planted = copy.deepcopy(matrix)
                    target = next(
                        candidate
                        for candidate in planted["risk_classes"]
                        if candidate["id"] == row["id"]
                    )
                    target["critical_paths"].remove(path)
                    self.assert_invalid(planted, f"missing critical path: {path}")

    def test_duplicate_ids_and_paths_are_rejected(self):
        matrix = complete_matrix()
        duplicate_class = copy.deepcopy(matrix)
        duplicate_class["risk_classes"].append(
            copy.deepcopy(duplicate_class["risk_classes"][0])
        )
        self.assert_invalid(duplicate_class, "duplicate risk class: pure-domain-error")

        duplicate_path = copy.deepcopy(matrix)
        duplicate_path["risk_classes"][1]["critical_paths"].append("parser")
        self.assert_invalid(duplicate_path, "duplicate critical path: parser")

    def test_a_critical_path_cannot_move_to_another_risk_class(self):
        matrix = complete_matrix()
        matrix["risk_classes"][0]["critical_paths"].remove("identity")
        matrix["risk_classes"][1]["critical_paths"].append("identity")

        self.assert_invalid(
            matrix,
            "pure-domain-error: critical_paths must be identity, scheduling",
        )

    def test_evidence_and_commands_cannot_disappear(self):
        matrix = complete_matrix()
        no_evidence = copy.deepcopy(matrix)
        no_evidence["risk_classes"][0]["primary_evidence"] = []
        self.assert_invalid(no_evidence, "pure-domain-error: primary_evidence must not be empty")

        no_command = copy.deepcopy(matrix)
        no_command["risk_classes"][0]["primary_evidence"][0]["command"] = ""
        self.assert_invalid(no_command, "pure-domain-error evidence 1: command must not be empty")

    def test_state_and_gap_shapes_are_honest(self):
        matrix = complete_matrix()
        covered_gap = copy.deepcopy(matrix)
        covered_gap["risk_classes"][0]["gap"] = "still broken"
        self.assert_invalid(covered_gap, "pure-domain-error: covered rows cannot declare a gap")

        gap_without_detail = copy.deepcopy(matrix)
        gap_without_detail["risk_classes"][2].pop("gap")
        self.assert_invalid(gap_without_detail, "data-loss: gap rows require gap detail")

        gap_without_owner = copy.deepcopy(matrix)
        gap_without_owner["risk_classes"][2].pop("roadmap_anchor")
        self.assert_invalid(
            gap_without_owner,
            "data-loss: gap rows require a roadmap_anchor",
        )

    def test_unknown_schema_vocabulary_is_rejected(self):
        matrix = complete_matrix()
        unknown_root = copy.deepcopy(matrix)
        unknown_root["version"] = 1
        self.assert_invalid(unknown_root, "root: unknown key: version")

        unknown_row = copy.deepcopy(matrix)
        unknown_row["risk_classes"][0]["coverage"] = "good"
        self.assert_invalid(unknown_row, "pure-domain-error: unknown key: coverage")

        unknown_evidence = copy.deepcopy(matrix)
        unknown_evidence["risk_classes"][0]["primary_evidence"][0]["job"] = "test"
        self.assert_invalid(
            unknown_evidence,
            "pure-domain-error evidence 1: unknown key: job",
        )

    def test_unknown_state_cadence_class_and_path_are_rejected(self):
        matrix = complete_matrix()
        bad_state = copy.deepcopy(matrix)
        bad_state["risk_classes"][0]["state"] = "mostly"
        self.assert_invalid(bad_state, "pure-domain-error: unknown state: mostly")

        bad_cadence = copy.deepcopy(matrix)
        bad_cadence["risk_classes"][0]["primary_evidence"][0]["cadence"] = "sometimes"
        self.assert_invalid(
            bad_cadence,
            "pure-domain-error evidence 1: unknown cadence: sometimes",
        )

        unknown_class = copy.deepcopy(matrix)
        unknown_class["risk_classes"][0]["id"] = "other"
        self.assert_invalid(unknown_class, "unknown risk class: other")

        unknown_path = copy.deepcopy(matrix)
        unknown_path["risk_classes"][0]["critical_paths"][0] = "other"
        self.assert_invalid(unknown_path, "unknown critical path: other")

    def test_only_schema_one_and_valid_json_are_accepted(self):
        matrix = complete_matrix()
        wrong_schema = copy.deepcopy(matrix)
        wrong_schema["schema"] = 2
        self.assert_invalid(wrong_schema, "schema must be 1")

        boolean_schema = copy.deepcopy(matrix)
        boolean_schema["schema"] = True
        self.assert_invalid(boolean_schema, "schema must be 1")

        float_schema = json.dumps(matrix).replace('"schema": 1,', '"schema": 1.0,', 1)
        self.assertIn('"schema": 1.0,', float_schema, "the float fixture replaced the schema")
        result = self.run_guard_text(float_schema)
        self.assertEqual(1, result.returncode, result.stdout + result.stderr)
        self.assertIn("risk-matrix: schema must be 1", result.stderr)

        result = self.run_guard_text("{")
        self.assertEqual(1, result.returncode, result.stdout + result.stderr)
        self.assertIn("risk-matrix: invalid JSON:", result.stderr)

    def test_commands_must_name_one_make_target(self):
        matrix = complete_matrix()
        bad_command = copy.deepcopy(matrix)
        bad_command["risk_classes"][0]["primary_evidence"][0]["command"] = (
            "cargo test"
        )
        self.assert_invalid(
            bad_command,
            "pure-domain-error evidence 1: command must be `make <target>`",
        )

    def test_renaming_each_named_make_target_turns_the_guard_red(self):
        matrix = complete_matrix()
        makefile = self.fixture_makefile(matrix)
        targets = {
            evidence["command"].split()[1]
            for row in matrix["risk_classes"]
            for evidence in row["primary_evidence"]
        }
        for target in targets:
            with self.subTest(target=target):
                renamed = "\n".join(
                    f"{target}-renamed:" if line == f"{target}:" else line
                    for line in makefile.splitlines()
                )
                self.assert_invalid(
                    matrix,
                    f"command does not resolve: make {target}",
                    renamed,
                    existing_paths=(target,),
                )


if __name__ == "__main__":
    unittest.main()
