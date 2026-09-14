#!/usr/bin/env python3
"""Validate the checked-in testing risk inventory and its Make commands."""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
DEFAULT_MATRIX = ROOT / "scripts" / "testing-risk-matrix.json"
DEFAULT_MAKEFILE = ROOT / "Makefile"
EXPECTED_PATHS = {
    "pure-domain-error": ("identity", "scheduling"),
    "parser-hostility": ("parser", "locator"),
    "data-loss": ("zip-receive", "store", "os-matrix"),
    "api-drift": ("api-contract",),
    "ui-control-no-op": ("web-mobile-controls",),
    "concurrency-race": ("soak",),
    "backend-cli-drift": ("backend-cli",),
    "ai-judgment": ("grader-calibration",),
    "release-defect": ("packaged-artifact",),
}
KNOWN_PATHS = {path for paths in EXPECTED_PATHS.values() for path in paths}
ROOT_KEYS = {"schema", "risk_classes"}
ROW_KEYS = {
    "id",
    "critical_paths",
    "primary_evidence",
    "state",
    "gap",
    "roadmap_anchor",
}
ROW_REQUIRED = {"id", "critical_paths", "primary_evidence", "state"}
EVIDENCE_KEYS = {"kind", "command", "cadence"}
STATES = {"covered", "partial", "gap"}
CADENCES = {"per-change", "per-push", "nightly", "release", "manual"}
TARGET = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]*")


def exact_keys(value, allowed, required, where, problems):
    for key in sorted(set(value) - allowed):
        problems.append(f"{where}: unknown key: {key}")
    for key in sorted(required - set(value)):
        problems.append(f"{where}: missing key: {key}")


def nonempty_string(value):
    return isinstance(value, str) and bool(value.strip())


def validate_evidence(risk_class, evidence, number, commands, problems):
    where = f"{risk_class} evidence {number}"
    if not isinstance(evidence, dict):
        problems.append(f"{where}: must be an object")
        return
    exact_keys(evidence, EVIDENCE_KEYS, EVIDENCE_KEYS, where, problems)
    for key in ("kind", "command", "cadence"):
        if key in evidence and not nonempty_string(evidence[key]):
            problems.append(f"{where}: {key} must not be empty")
    cadence = evidence.get("cadence")
    if nonempty_string(cadence) and cadence not in CADENCES:
        problems.append(f"{where}: unknown cadence: {cadence}")
    command = evidence.get("command")
    if not nonempty_string(command):
        return
    words = command.split()
    if len(words) != 2 or words[0] != "make" or not TARGET.fullmatch(words[1]):
        problems.append(f"{where}: command must be `make <target>`")
        return
    commands.add(command)


def validate_row(row, seen_classes, seen_paths, commands, problems):
    if not isinstance(row, dict):
        problems.append("risk class row must be an object")
        return
    risk_class = row.get("id")
    where = risk_class if nonempty_string(risk_class) else "risk class row"
    exact_keys(row, ROW_KEYS, ROW_REQUIRED, where, problems)
    if not nonempty_string(risk_class):
        problems.append("risk class row: id must not be empty")
        return
    if risk_class not in EXPECTED_PATHS:
        problems.append(f"unknown risk class: {risk_class}")
    if risk_class in seen_classes:
        problems.append(f"duplicate risk class: {risk_class}")
    seen_classes.add(risk_class)

    paths = row.get("critical_paths")
    row_paths = []
    if not isinstance(paths, list) or not paths:
        problems.append(f"{risk_class}: critical_paths must not be empty")
    else:
        for path in paths:
            if not nonempty_string(path):
                problems.append(f"{risk_class}: critical path must not be empty")
                continue
            row_paths.append(path)
            if path not in KNOWN_PATHS:
                problems.append(f"unknown critical path: {path}")
            if path in seen_paths:
                problems.append(f"duplicate critical path: {path}")
            seen_paths.add(path)
    expected_paths = EXPECTED_PATHS.get(risk_class)
    if expected_paths is not None and set(row_paths) != set(expected_paths):
        problems.append(
            f"{risk_class}: critical_paths must be {', '.join(expected_paths)}"
        )

    evidence_rows = row.get("primary_evidence")
    if not isinstance(evidence_rows, list) or not evidence_rows:
        problems.append(f"{risk_class}: primary_evidence must not be empty")
    else:
        for number, evidence in enumerate(evidence_rows, start=1):
            validate_evidence(risk_class, evidence, number, commands, problems)

    state = row.get("state")
    if not nonempty_string(state):
        problems.append(f"{risk_class}: state must not be empty")
    elif state not in STATES:
        problems.append(f"{risk_class}: unknown state: {state}")
    gap = row.get("gap")
    anchor = row.get("roadmap_anchor")
    if state == "covered":
        if "gap" in row:
            problems.append(f"{risk_class}: covered rows cannot declare a gap")
        if "roadmap_anchor" in row:
            problems.append(f"{risk_class}: covered rows cannot declare a roadmap_anchor")
    elif state in {"partial", "gap"}:
        if not nonempty_string(gap):
            problems.append(f"{risk_class}: {state} rows require gap detail")
        if not nonempty_string(anchor):
            problems.append(f"{risk_class}: {state} rows require a roadmap_anchor")


def validate_matrix(matrix):
    problems = []
    commands = set()
    if not isinstance(matrix, dict):
        return ["root must be an object"], commands, 0
    exact_keys(matrix, ROOT_KEYS, ROOT_KEYS, "root", problems)
    schema = matrix.get("schema")
    # bool subclasses int, so isinstance would still let JSON true through.
    if type(schema) is not int or schema != 1:
        problems.append("schema must be 1")
    rows = matrix.get("risk_classes")
    if not isinstance(rows, list):
        problems.append("risk_classes must be an array")
        return problems, commands, 0

    seen_classes = set()
    seen_paths = set()
    for row in rows:
        validate_row(row, seen_classes, seen_paths, commands, problems)

    for risk_class in EXPECTED_PATHS:
        if risk_class not in seen_classes:
            problems.append(f"missing risk class: {risk_class}")
    for expected in EXPECTED_PATHS.values():
        for path in expected:
            if path not in seen_paths:
                problems.append(f"missing critical path: {path}")

    return problems, commands, len(seen_paths)


def validate_commands(commands, makefile):
    problems = []
    try:
        makefile_text = makefile.read_text(encoding="utf-8")
    except OSError as error:
        return [f"cannot read Makefile: {error}"]
    defined_targets = {
        match.group(1)
        for line in makefile_text.splitlines()
        if (match := re.match(r"^([A-Za-z0-9][A-Za-z0-9_.-]*):(?!=)", line))
    }
    for command in sorted(commands):
        target = command.split()[1]
        if target not in defined_targets:
            problems.append(f"command does not resolve: {command}")
            continue
        result = subprocess.run(
            [
                "make",
                "--no-print-directory",
                "--dry-run",
                "--file",
                str(makefile),
                target,
            ],
            cwd=makefile.parent,
            text=True,
            capture_output=True,
            check=False,
        )
        if result.returncode != 0:
            problems.append(f"command does not resolve: {command}")
    return problems


def arguments():
    parser = argparse.ArgumentParser()
    parser.add_argument("--matrix", type=Path, default=DEFAULT_MATRIX)
    parser.add_argument("--makefile", type=Path, default=DEFAULT_MAKEFILE)
    return parser.parse_args()


def main():
    args = arguments()
    try:
        matrix = json.loads(args.matrix.read_text(encoding="utf-8"))
    except OSError as error:
        print(f"risk-matrix: cannot read matrix: {error}", file=sys.stderr)
        return 1
    except json.JSONDecodeError as error:
        print(f"risk-matrix: invalid JSON: {error}", file=sys.stderr)
        return 1

    problems, commands, path_count = validate_matrix(matrix)
    if not problems:
        problems.extend(validate_commands(commands, args.makefile.resolve()))
    for problem in problems:
        print(f"risk-matrix: {problem}", file=sys.stderr)
    if problems:
        return 1
    print(
        f"risk-matrix: {len(matrix['risk_classes'])} risk classes, "
        f"{path_count} critical paths, {len(commands)} runnable commands"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
