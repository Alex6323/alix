#!/usr/bin/env python3
"""Keep the tracked landing-page media set complete and intentionally small."""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
MEDIA_DIR = REPO_ROOT / "site" / "img"
CAPTURE = REPO_ROOT / "e2e" / "shots" / "capture.cjs"
SHOTS_TABLE = re.compile(r"const SHOTS = \[(.*?)\];", re.S)
SHOT_ROW = re.compile(r'\[\d+, "([^"]+)", shot\d+\]')
SITE_PAGE = REPO_ROOT / "site" / "index.html"
PAGE_SHOT = re.compile(r'src="img/(shot-[^"]+\.webp)"')
README = REPO_ROOT / "README.md"
README_SHOT = re.compile(r"/img/(shot-[^\s)\"]+\.webp)")
MEDIA_BUDGET_BYTES = 3 * 1024 * 1024 // 2


def registered_shots() -> set[str]:
    table = SHOTS_TABLE.search(CAPTURE.read_text(encoding="utf-8"))
    if table is None:
        return set()
    return set(SHOT_ROW.findall(table.group(1)))


def referenced_shots() -> list[str]:
    return PAGE_SHOT.findall(SITE_PAGE.read_text(encoding="utf-8"))


def readme_shots() -> set[str]:
    return set(README_SHOT.findall(README.read_text(encoding="utf-8")))


def main() -> int:
    media = sorted(path for path in MEDIA_DIR.rglob("*") if path.is_file())
    shot_names = {path.name for path in media if path.name.startswith("shot-")}
    expected = registered_shots()
    missing = sorted(expected - shot_names)
    unexpected = sorted(shot_names - expected)
    total_bytes = sum(path.stat().st_size for path in media)

    referenced = referenced_shots()
    repeated = sorted({name for name in referenced if referenced.count(name) > 1})
    unregistered_in_readme = sorted(readme_shots() - expected)

    errors = []
    if repeated:
        errors.append(f"the landing page repeats a screenshot: {', '.join(repeated)}")
    if set(referenced) != expected:
        errors.append(
            "the landing page and the capture registry disagree: "
            f"referenced but unregistered {sorted(set(referenced) - expected)}, "
            f"registered but unreferenced {sorted(expected - set(referenced))}"
        )
    if unregistered_in_readme:
        errors.append(
            "the README references unregistered screenshots: "
            f"{', '.join(unregistered_in_readme)}"
        )
    if not expected:
        errors.append(f"no capture registry found in {CAPTURE}")
    if missing:
        errors.append(f"missing carousel screenshots: {', '.join(missing)}")
    if unexpected:
        errors.append(f"unexpected carousel files: {', '.join(unexpected)}")
    if total_bytes > MEDIA_BUDGET_BYTES:
        errors.append(
            f"site media uses {total_bytes / 1024 / 1024:.2f} MiB, "
            f"over the {MEDIA_BUDGET_BYTES / 1024 / 1024:.2f} MiB budget"
        )

    print(
        f"site media: {len(media)} files, {total_bytes / 1024 / 1024:.2f} MiB "
        f"/ {MEDIA_BUDGET_BYTES / 1024 / 1024:.2f} MiB"
    )
    for error in errors:
        print(f"site-media-check: {error}", file=sys.stderr)
    return int(bool(errors))


if __name__ == "__main__":
    raise SystemExit(main())
