#!/usr/bin/env python3
"""Every buildable unit is gated somewhere, and the omissions are named here.

`make check` compiles the root crate and nothing else. Three separate units
turned main red in one day because that was remembered rather than checked:
the mobile bridge crate, the GFM corpus harness, and the Playwright suite.

This walks the repository for units a change can break, then requires each to
be either reachable from a local gate (`check` or `preflight`) or listed in
CI_ONLY below with a reason AND a marker that proves a workflow runs it. A new
crate or suite is in neither, so it fails until someone decides which.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
MAKEFILE = REPO_ROOT / "Makefile"
WORKFLOWS = REPO_ROOT / ".github" / "workflows"
LOCAL_GATES = ("check", "preflight")

SKIP_DIRS = {"target", "node_modules", ".git", "build", ".dart_tool"}

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
        "cargo-fuzz",
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


def units() -> list[str]:
    """Repo-relative paths of every unit a change can break."""
    found = []
    for manifest in REPO_ROOT.rglob("Cargo.toml"):
        relative = manifest.relative_to(REPO_ROOT)
        if SKIP_DIRS & set(relative.parts):
            continue
        found.append(relative.as_posix())
    for suite in ("e2e/unit", "e2e/tests"):
        if (REPO_ROOT / suite).is_dir():
            found.append(suite)
    for app in sorted((REPO_ROOT / "mobile").glob("*/test")):
        found.append(app.relative_to(REPO_ROOT).as_posix())
    return sorted(found)


def workflow_text() -> str:
    return "\n".join(path.read_text() for path in sorted(WORKFLOWS.glob("*.yml")))


def main() -> int:
    recipes = reachable_recipes(make_targets(MAKEFILE.read_text()), LOCAL_GATES)
    workflows = workflow_text()
    ungated, stale = [], []

    for unit in units():
        # The root manifest is what a bare `cargo` command builds.
        if unit == "Cargo.toml":
            if "cargo test" in recipes:
                continue
            ungated.append(f"{unit} (no bare `cargo test` reachable from a local gate)")
            continue
        if unit in recipes:
            continue
        if unit in CI_ONLY:
            reason, marker = CI_ONLY[unit]
            if marker not in workflows:
                stale.append(f"{unit}: no workflow runs `{marker}` any more ({reason})")
            continue
        ungated.append(unit)

    for unit in sorted(CI_ONLY):
        if unit not in units():
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

    print(f"gate-coverage: {len(units())} units, each locally gated or named CI-only")
    return 0


if __name__ == "__main__":
    sys.exit(main())
