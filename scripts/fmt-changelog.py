#!/usr/bin/env python3
"""Normalize CHANGELOG.md: wrap to the ~80-column house width, one blank line
between entries.

Deliberately conservative (the fmt-roadmap.py sibling): a line is wrapped ONLY
if it exceeds the width, so short lines and hand-made breaks pass through
byte-identical; the only other changes are blank-line normalization between
entries, so running this never churns deliberate formatting.

  * bullet entries (``- `` at any indent) continue aligned under their text
  * already-indented continuation lines keep their own indent
  * consecutive top-level entries are separated by exactly one blank line
    (inserted when missing; runs of blank lines collapse to one)
  * headings, link-reference definitions, and fenced code blocks are never
    touched, and a long run without spaces (a URL, a hash) is left whole

Stdlib only. Usage: fmt-changelog.py [FILE ...]   (default: CHANGELOG.md)
"""

import re
import sys
import textwrap
from pathlib import Path

WIDTH = 80

BULLET_RE = re.compile(r"^(\s*)- ")
LINK_DEF_RE = re.compile(r"^\[[^\]]+\]:\s")


def wrap_line(line: str) -> list[str]:
    if len(line) <= WIDTH:
        return [line]
    stripped = line.lstrip(" ")
    if stripped.startswith("#") or LINK_DEF_RE.match(line):
        return [line]
    m = BULLET_RE.match(line)
    if m:
        subsequent = " " * (len(m.group(1)) + 2)
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


def format_text(text: str) -> str:
    out: list[str] = []
    fenced = False
    for line in text.split("\n"):
        if line.lstrip().startswith("```"):
            fenced = not fenced
            out.append(line)
        elif fenced:
            out.append(line)
        elif not line:
            if out and out[-1]:
                out.append(line)
        else:
            if line.startswith("- ") and out and out[-1] and not out[-1].startswith("#"):
                out.append("")
            out.extend(wrap_line(line))
    return "\n".join(out)


def main() -> int:
    files = [Path(p) for p in sys.argv[1:]] or [Path("CHANGELOG.md")]
    changed = 0
    for path in files:
        text = path.read_text()
        formatted = format_text(text)
        if formatted != text:
            path.write_text(formatted)
            changed += 1
            print(f"formatted {path}")
    print(f"{changed} file(s) changed" if changed else "already formatted")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
