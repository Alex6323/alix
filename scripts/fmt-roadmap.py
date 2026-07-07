#!/usr/bin/env python3
"""Wrap over-long lines in ROADMAP.md-style markdown files.

Wrap-only, deliberately conservative: a line is touched ONLY if it exceeds the
width — short lines (including hand-made breaks and alignment) pass through
byte-identical, so running this never churns deliberate formatting. Long lines
wrap onto the roadmap's continuation convention:

  * roadmap items (``* [`` at column 0) continue at the house 13-space indent
  * already-indented continuation lines keep their own indent
  * headings, table rows, and fenced code blocks are never touched

Stdlib only. Usage: fmt-roadmap.py [FILE ...]   (default: ROADMAP.md)
"""

import sys
import textwrap
from pathlib import Path

WIDTH = 100
ITEM_INDENT = " " * 13  # continuation alignment used throughout ROADMAP.md


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


def main() -> None:
    paths = [Path(a) for a in sys.argv[1:]] or [Path("ROADMAP.md")]
    for path in paths:
        if not path.is_file():
            sys.exit(f"fmt-roadmap: no such file: {path}")
        n = format_file(path)
        print(f"{path}: wrapped {n} over-long line(s)")


if __name__ == "__main__":
    main()
