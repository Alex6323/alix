#!/usr/bin/env python3
"""Validate and reconcile cargo-mutants output against the argued allowlist.

The pin check rejects line-pinned entries that no longer name a current
mutant. Reconciliation reads mutants.out/missed.txt and mutants.out/timeout.txt,
prints every survivor as ALLOWED or UNREGISTERED, and exits 0 only when every
one is allowed. Timeouts are reconciled the same way as misses because
cargo-mutants reports a timeout in preference to a miss (exit 3 beats
exit 2), so a shard with even one unargued timeout stays red regardless
of its misses.

Allowlist entries that matched nothing are reported informationally: on a
diff-scoped run most entries are simply out of scope, but on a whole-tree
sweep of their own shard that note is the removal reminder.
"""

import pathlib
import re
import sys

LINE_RE = re.compile(r"^(?P<file>[^:]+):(?P<line>\d+):\d+: (?P<desc>.*)$")
PIN_RE = re.compile(r"^(?P<file>[^:]+):(?P<line>\d+): (?P<desc>.*)$")


def read_lines(path: pathlib.Path) -> list[str]:
    try:
        return [line.strip() for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    except OSError:
        return []


def allowlist_entries(path: pathlib.Path, *, required: bool = False) -> list[str]:
    lines = (
        path.read_text(encoding="utf-8").splitlines()
        if required
        else read_lines(path)
    )
    return [
        line.strip()
        for line in lines
        if not line.lstrip().startswith("#")
    ]


def check_pins(mutant_lines: list[str], allowlist_path: pathlib.Path) -> int:
    current = {
        (match["file"], match["line"], match["desc"])
        for line in mutant_lines
        if (match := LINE_RE.match(line)) is not None
    }
    try:
        entries = allowlist_entries(allowlist_path, required=True)
    except OSError as error:
        print(f"allowlist: cannot read {allowlist_path}: {error}", file=sys.stderr)
        return 1

    stale = []
    for entry in entries:
        match = PIN_RE.match(entry)
        if match is not None and (
            match["file"],
            match["line"],
            match["desc"],
        ) not in current:
            stale.append(entry)

    for entry in stale:
        print(f"allowlist: {entry} names no current mutant", file=sys.stderr)
    return int(bool(stale))


def reconcile(out_dir: str, allowlist_path: str) -> int:
    entries = allowlist_entries(pathlib.Path(allowlist_path))
    out = pathlib.Path(out_dir)
    survivors = [("missed", m) for m in read_lines(out / "missed.txt")]
    survivors += [("timeout", t) for t in read_lines(out / "timeout.txt")]

    used = set()
    unregistered = []
    for kind, survivor in survivors:
        match = LINE_RE.match(survivor)
        key = None
        if match:
            broad = f"{match['file']}: {match['desc']}"
            narrow = f"{match['file']}:{match['line']}: {match['desc']}"
            key = next((e for e in (narrow, broad) if e in entries), None)
        if key is None:
            unregistered.append((kind, survivor))
        else:
            used.add(key)
            print(f"allowed {kind}:  {survivor}")

    for entry in entries:
        if entry not in used:
            print(f"note: allowlist entry matched nothing this run: {entry}")

    if unregistered:
        for kind, survivor in unregistered:
            print(f"UNREGISTERED {kind}: {survivor}")
        print(
            f"mutants-allowed: {len(unregistered)} unregistered survivor(s); "
            "kill them or argue them into scripts/mutants-allowlist.txt"
        )
        return 1
    print(f"mutants-allowed: all {len(survivors)} survivor(s) are argued residuals")
    return 0


if __name__ == "__main__":
    if len(sys.argv) == 4 and sys.argv[1] == "--check-pins":
        mutant_lines = read_lines(pathlib.Path(sys.argv[2]))
        sys.exit(check_pins(mutant_lines, pathlib.Path(sys.argv[3])))
    if len(sys.argv) == 3:
        sys.exit(reconcile(sys.argv[1], sys.argv[2]))
    sys.exit(
        "usage: mutants_allowed.py <mutants.out dir> <allowlist>\n"
        "       mutants_allowed.py --check-pins <mutants list> <allowlist>"
    )
