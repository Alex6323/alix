#!/usr/bin/env python3
"""Render the grader-calibration matrix from a `make calibrate` log.

Reads the `calibrate: probe=... backend=... requested=... observed=...
verdict=...` lines the harness prints, groups them by probe and backend
row, and prints one Markdown table under a provenance header:

    python3 scripts/calibrate_matrix.py <log> [--tree <text>] [--run <text>]
"""
import argparse
import re
import sys
from collections import OrderedDict

CELL = re.compile(
    r"calibrate: probe=(?P<probe>\S+) backend=(?P<backend>\S+) "
    r"requested=(?P<requested>\S+) observed=(?P<observed>\S+) verdict=(?P<verdict>\S+)"
)
TEST = re.compile(r"^test (?P<name>\w+) \.\.\.")
RESULT = re.compile(r"^test result: (?P<summary>.+)$")


def parse(text):
    rows = OrderedDict()
    columns = []
    tests = {}
    observed = set()
    summary = None
    current_test = None
    for line in text.splitlines():
        m = TEST.match(line)
        if m:
            current_test = m.group("name")
        m = RESULT.match(line)
        if m:
            summary = m.group("summary")
        for m in CELL.finditer(line):
            column = f"{m.group('backend')} / {m.group('requested')}"
            if column not in columns:
                columns.append(column)
            rows.setdefault(m.group("probe"), OrderedDict())[column] = m.group("verdict")
            tests.setdefault(m.group("probe"), current_test)
            observed.add(m.group("observed"))
    return rows, columns, tests, observed, summary


def render(rows, columns, tests, observed, summary, tree, run):
    out = ["# Grader calibration matrix", ""]
    if run:
        out.append(f"- Run: {run}")
    if tree:
        out.append(f"- Tree: {tree}")
    if summary:
        out.append(f"- Result: {summary}")
    cells = sum(len(r) for r in rows.values())
    out.append(f"- Cells: {cells} ({len(rows)} probes x {len(columns)} backend rows)")
    out.append(f"- Served model as reported by the CLI: {', '.join(sorted(observed)) or 'none'}")
    out.append("")
    out.append(
        "Each cell is the grader's verdict on the probe answer for that backend "
        "row (the CLI default and a floor row per backend). A test passes when "
        "every cell lands in the class its name demands: Pass for the \"passes\" "
        "tests, anything but Pass for the \"does not pass\" tests."
    )
    out.append("")
    out.append("| probe | test | " + " | ".join(columns) + " |")
    out.append("|---|---|" + "|".join("---" for _ in columns) + "|")
    for probe, verdicts in rows.items():
        cells = " | ".join(verdicts.get(c, "?") for c in columns)
        out.append(f"| {probe} | {tests.get(probe) or '?'} | {cells} |")
    return "\n".join(out) + "\n"


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("log")
    parser.add_argument("--tree", default="", help="what was calibrated (commit or candidate)")
    parser.add_argument("--run", default="", help="when and how the run happened")
    args = parser.parse_args(argv)
    with open(args.log, encoding="utf-8", errors="replace") as handle:
        rows, columns, tests, observed, summary = parse(handle.read())
    if not rows:
        sys.stderr.write("calibrate_matrix: no calibrate lines found\n")
        return 1
    sys.stdout.write(render(rows, columns, tests, observed, summary, args.tree, args.run))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
