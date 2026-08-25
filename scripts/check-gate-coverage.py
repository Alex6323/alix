#!/usr/bin/env python3
"""Every buildable unit is gated somewhere, and the omissions are named here.

`make check` compiles the root crate and nothing else. Three separate units
turned main red in one day because that was remembered rather than checked:
the mobile bridge crate, the GFM corpus harness, and the Playwright suite.

This walks the repository for units a change can break, then requires each to
be either reachable from a local gate (`check` or `preflight`) or listed in
CI_ONLY below with a reason AND a marker that proves a workflow runs it. A new
crate or suite is in neither, so it fails until someone decides which.

A unit is discovered from evidence rather than from a list of known places: a
project manifest, a directory named for tests, or a directory holding files
named the way test files are named. The residual is the naming conventions
themselves, so a suite whose files match no pattern below is still invisible.
"""

from __future__ import annotations

import os
import re
import shlex
import sys
from fnmatch import fnmatch
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
MAKEFILE = REPO_ROOT / "Makefile"
WORKFLOWS = REPO_ROOT / ".github" / "workflows"
LOCAL_GATES = ("check", "preflight")

SKIP_DIRS = {
    "target",
    "node_modules",
    ".git",
    "build",
    ".dart_tool",
    "__pycache__",
    ".venv",
}
MANIFESTS = ("Cargo.toml", "pyproject.toml")
TEST_DIR_NAMES = {"test", "tests", "spec", "__tests__", "integration_test"}
TEST_FILE_PATTERNS = (
    "test_*.py",
    "*_test.py",
    "*.test.mjs",
    "*.test.js",
    "*.test.ts",
    "*.spec.mjs",
    "*.spec.js",
    "*.spec.ts",
    "*_test.dart",
    "*_test.go",
)
CARGO_SUBDIRS = {"tests", "benches", "examples"}

# A marker must run in a workflow that gates a push to main or a pull request.
# These units are the deliberate exception, where a schedule is the point.
SCHEDULED = {"fuzz/Cargo.toml"}

# A unit `make check` and `make preflight` deliberately do not run. Each needs
# the workflow marker that proves something else does; a stale exception whose
# job was deleted fails here rather than going quiet.
CI_ONLY = {
    "e2e/tests": (
        "Playwright drives real browsers: minutes plus a browser download, so "
        "CI owns it (the preflight comment records the decision).",
        "make e2e",
    ),
    "e2e/unit": (
        "The JavaScript unit suite runs in its own blocking CI job.",
        "unit-js",
    ),
    "fuzz/Cargo.toml": (
        "cargo-fuzz needs nightly and runs on a weekly schedule, never per push.",
        "make fuzz-stamp",
    ),
    "mobile/alix/integration_test": (
        "The integration suite needs a Linux window, so preflight runs the "
        "unit half and CI runs this one under xvfb.",
        "flutter test integration_test",
    ),
    "orchestrator/pyproject.toml": (
        "The orchestrator is a standalone uv project outside Cargo, "
        "type-checked in its own CI job.",
        "--config-file orchestrator/pyproject.toml",
    ),
    "orchestrator/tests": (
        "Same project: its suite runs under uv in CI, not from the Makefile.",
        "-s orchestrator/tests",
    ),
}


def make_targets(text: str) -> dict[str, tuple[list[str], list[str]]]:
    """Map each target to (prerequisites, recipe lines)."""
    targets: dict[str, tuple[list[str], list[str]]] = {}
    current: str | None = None
    for line in text.splitlines():
        if line.startswith("\t"):
            if current:
                targets[current][1].append(line)
            continue
        match = re.match(r"^([A-Za-z0-9_.-]+):(?!=)\s*(.*)$", line)
        if match:
            current = match.group(1)
            targets.setdefault(current, ([], []))
            targets[current][0].extend(match.group(2).split())
        elif line.strip() and not line.startswith("#"):
            current = None
    return targets


def reachable_recipes(targets: dict[str, tuple[list[str], list[str]]], roots) -> str:
    """Every recipe line of the roots and of what they transitively invoke."""
    seen: set[str] = set()
    lines: list[str] = []
    queue = list(roots)
    while queue:
        name = queue.pop()
        if name in seen or name not in targets:
            continue
        seen.add(name)
        prerequisites, recipe = targets[name]
        lines.extend(expand_cd(line) for line in recipe)
        queue.extend(prerequisites)
        for line in recipe:
            queue.extend(re.findall(r"\$\(MAKE\)\s+([A-Za-z0-9_.-]+)", line))
    return "\n".join(lines)


def expand_cd(line: str) -> str:
    """`cd mobile/alix && flutter test test/` also names `mobile/alix/test/`."""
    match = re.match(r"^\t(?:@|-)?cd\s+(\S+)\s*&&\s*(.*)$", line)
    if not match:
        return line
    directory, rest = match.groups()
    joined = re.sub(r"(?<![\w./-])([\w.-]+/[\w./-]*)", rf"{directory}/\1", rest)
    return f"{line}\n{joined}"


def commands(text: str) -> list[list[str]]:
    """Recipe text as argument lists, so a path counts only where it is one."""
    parsed: list[list[str]] = []
    for line in text.splitlines():
        line = line.lstrip("\t").lstrip("@-").rstrip("\\")
        for part in re.split(r"&&|\|\||;|(?<!\|)\|(?!\|)", line):
            try:
                tokens = shlex.split(part, comments=False)
            except ValueError:
                tokens = part.split()
            if tokens:
                parsed.append(tokens)
    return parsed


def names_path(command: list[str], unit: str) -> bool:
    return any(token.rstrip("/") == unit for token in command)


def is_whole_crate_test(command: list[str]) -> bool:
    """`cargo test` over the root crate, which a --manifest-path run is not."""
    words = [token for token in command if "=" not in token or token.startswith("-")]
    if not words or words[0] != "cargo":
        return False
    subcommands = [token for token in words[1:] if not token.startswith("-")]
    if not subcommands or subcommands[0] not in {"test", "nextest"}:
        return False
    return not any(token.split("=")[0] == "--manifest-path" for token in command)


def units() -> list[str]:
    """Repo-relative paths of every unit a change can break."""
    manifests, test_dirs = walk()
    crates = {
        Path(manifest).parent.as_posix()
        for manifest in manifests
        if Path(manifest).name == "Cargo.toml"
    }
    found = set(manifests)
    for directory in test_dirs:
        path = Path(directory)
        if path.name in CARGO_SUBDIRS and path.parent.as_posix() in crates:
            continue
        found.add(directory)
    return sorted(found)


def walk() -> tuple[set[str], set[str]]:
    """One pruned pass: manifest paths, and directories that hold tests."""
    manifests: set[str] = set()
    test_dirs: set[str] = set()
    for parent, directories, files in os.walk(REPO_ROOT):
        directories[:] = [name for name in directories if name not in SKIP_DIRS]
        relative = Path(parent).relative_to(REPO_ROOT)
        for name in files:
            if name in MANIFESTS:
                manifests.add((relative / name).as_posix())
        if relative == Path("."):
            continue
        holds_tests = relative.name in TEST_DIR_NAMES or any(
            fnmatch(name, pattern) for name in files for pattern in TEST_FILE_PATTERNS
        )
        if holds_tests:
            test_dirs.add(relative.as_posix())
    return manifests, test_dirs


def workflow_text(blocking_only: bool = False) -> str:
    """Only what a workflow executes: `run:` scalars and blocks, no prose."""
    executable: list[str] = []
    for path in sorted(WORKFLOWS.glob("*.yml")) + sorted(WORKFLOWS.glob("*.yaml")):
        text = path.read_text()
        if blocking_only and not gates_a_push(text):
            continue
        executable.extend(run_values(text))
    return "\n".join(executable)


def gates_a_push(text: str) -> bool:
    """A workflow that runs on a push to main or a pull request, not a tag."""
    triggers = trigger_block(text)
    if "pull_request" in triggers:
        return True
    return "push" in triggers and set(triggers["push"]) != {"tags"}


def trigger_block(text: str) -> dict[str, list[str]]:
    triggers: dict[str, list[str]] = {}
    lines = text.splitlines()
    for index, line in enumerate(lines):
        inline = re.match(r"^on:\s*(.*)$", line)
        if not inline:
            continue
        for name in re.findall(r"[A-Za-z_]+", inline.group(1)):
            triggers[name] = []
        current: str | None = None
        for body in lines[index + 1 :]:
            if body.strip() and not body.startswith(" "):
                break
            indent = len(body) - len(body.lstrip())
            key = re.match(r"^\s*([A-Za-z_]+):", body)
            if not key or not body.strip() or body.lstrip().startswith("#"):
                continue
            if indent == 2:
                current = key.group(1)
                triggers.setdefault(current, [])
            elif indent == 4 and current:
                triggers[current].append(key.group(1))
        break
    return triggers


def run_values(text: str) -> list[str]:
    values: list[str] = []
    lines = text.splitlines()
    index = 0
    while index < len(lines):
        match = re.match(r"^(\s*)(?:-\s+)?run:\s*(.*?)\s*$", lines[index])
        index += 1
        if not match:
            continue
        indent, inline = match.groups()
        if inline and not re.fullmatch(r"[|>][-+0-9]*", inline):
            values.append(inline)
            continue
        while index < len(lines):
            body = lines[index]
            if body.strip() and len(body) - len(body.lstrip()) <= len(indent):
                break
            if not body.lstrip().startswith("#"):
                values.append(body)
            index += 1
    return values


def main() -> int:
    targets = make_targets(MAKEFILE.read_text())
    recipes = commands(reachable_recipes(targets, LOCAL_GATES))
    blocking = workflow_text(blocking_only=True)
    scheduled = workflow_text()
    inventory = units()
    ungated, stale = [], []

    for unit in inventory:
        if unit == "Cargo.toml":
            if any(is_whole_crate_test(command) for command in recipes):
                continue
            ungated.append(f"{unit} (no bare `cargo test` reachable from a local gate)")
            continue
        if any(names_path(command, unit) for command in recipes):
            continue
        if unit in CI_ONLY:
            reason, marker = CI_ONLY[unit]
            workflows = scheduled if unit in SCHEDULED else blocking
            if marker not in workflows:
                where = "scheduled" if unit in SCHEDULED else "push-gating"
                stale.append(
                    f"{unit}: no {where} workflow runs `{marker}` any more ({reason})"
                )
            continue
        ungated.append(unit)

    for unit in sorted(CI_ONLY):
        if unit not in inventory:
            stale.append(f"{unit}: listed as CI-only but no longer exists")

    for problem in stale:
        print(f"gate-coverage: {problem}", file=sys.stderr)
    for unit in ungated:
        print(
            f"gate-coverage: {unit} is built by neither `make check` nor "
            "`make preflight`, and is not listed as CI-only",
            file=sys.stderr,
        )
    if ungated or stale:
        print(
            "\nAdd it to a local gate, or to CI_ONLY in scripts/check-gate-coverage.py "
            "with a reason and the workflow marker that proves CI runs it.",
            file=sys.stderr,
        )
        return 1

    print(f"gate-coverage: {len(inventory)} units, each locally gated or named CI-only")
    return 0


if __name__ == "__main__":
    sys.exit(main())
