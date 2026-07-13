#!/usr/bin/env python3
"""Wrap over-long lines in ROADMAP.md-style markdown files.

Wrap-only, deliberately conservative: a line is touched ONLY if it exceeds the
width — short lines (including hand-made breaks and alignment) pass through
byte-identical, so running this never churns deliberate formatting. Long lines
wrap onto the roadmap's continuation convention:

  * roadmap items (``* [`` at column 0) continue at the house 13-space indent
  * already-indented continuation lines keep their own indent
  * headings, table rows, and fenced code blocks are never touched

With ``--stats`` it reports instead of formats (read-only): item counts by
state (done ``[x]`` / partial ``[~]`` / open ``[ ]``) and the open items split
by priority (P0-P3, ``??``). This is the deterministic half of a roadmap
audit; whether an open item is secretly already shipped still needs a reader.

Stdlib only. Usage: fmt-roadmap.py [--stats] [FILE ...]   (default: ROADMAP.md)
"""

import re
import sys
import textwrap
from pathlib import Path

WIDTH = 100
ITEM_INDENT = " " * 13  # continuation alignment used throughout ROADMAP.md

# An item header: `* [ ]` / `* [x]` / `* [~]` at column 0, then a dash run and
# the priority token (P0-P3, `--` for done/none, `??` for unprioritized).
ITEM_RE = re.compile(r"^\* \[([ x~])\]\s*-+\s*(P[0-3]|\?\?|--)")


def wrap_line(line: str) -> list[str]:
    if len(line) <= WIDTH:
        return [line]
    stripped = line.lstrip(" ")
    if stripped.startswith("#") or stripped.startswith("|"):
        return [line]  # headings and table rows stay whole
    if line.startswith("* ["):
        subsequent = ITEM_INDENT
    else:
        subsequent = line[: len(line) - len(stripped)]
    wrapped = textwrap.wrap(
        line,
        width=WIDTH,
        subsequent_indent=subsequent,
        break_long_words=False,
        break_on_hyphens=False,
    )
    return wrapped or [line]


def format_file(path: Path) -> int:
    out: list[str] = []
    wrapped_count = 0
    in_fence = False
    for line in path.read_text().splitlines():
        if line.lstrip().startswith("```"):
            in_fence = not in_fence
            out.append(line)
            continue
        if in_fence:
            out.append(line)
            continue
        pieces = wrap_line(line)
        if len(pieces) > 1:
            wrapped_count += 1
        out.extend(pieces)
    path.write_text("\n".join(out) + "\n")
    return wrapped_count


def stats_file(path: Path) -> None:
    states = {" ": 0, "x": 0, "~": 0}
    open_by_priority: dict[str, int] = {}
    for line in path.read_text().splitlines():
        m = ITEM_RE.match(line)
        if not m:
            continue
        state, priority = m.groups()
        states[state] += 1
        if state == " ":
            open_by_priority[priority] = open_by_priority.get(priority, 0) + 1
    total = sum(states.values())
    print(
        f"{path}: {total} items: {states['x']} done"
        f" · {states['~']} partial · {states[' ']} open"
    )
    order = ["P0", "P1", "P2", "P3", "??", "--"]
    split = " · ".join(
        f"{p} {open_by_priority[p]}" for p in order if p in open_by_priority
    )
    print(f"open by priority: {split or 'none'}")


def main() -> None:
    args = sys.argv[1:]
    stats = "--stats" in args
    paths = [Path(a) for a in args if a != "--stats"] or [Path("ROADMAP.md")]
    for path in paths:
        if not path.is_file():
            sys.exit(f"fmt-roadmap: no such file: {path}")
        if stats:
            stats_file(path)
        else:
            n = format_file(path)
            print(f"{path}: wrapped {n} over-long line(s)")


if __name__ == "__main__":
    main()
