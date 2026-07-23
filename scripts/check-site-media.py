#!/usr/bin/env python3
"""Keep the tracked landing-page media set complete and intentionally small."""

from __future__ import annotations

import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
MEDIA_DIR = REPO_ROOT / "site" / "img"
MEDIA_BUDGET_BYTES = 3 * 1024 * 1024 // 2
EXPECTED_SHOTS = {
    "shot-1-verify.webp",
    "shot-2-tutor.webp",
    "shot-3-modes.webp",
    "shot-4-exam.webp",
    "shot-5-augment.webp",
    "shot-6-trace.webp",
    "shot-7-picker.webp",
    "shot-8-topology.webp",
    "shot-9-themes.webp",
    "shot-10-kids.webp",
}


def main() -> int:
    media = sorted(path for path in MEDIA_DIR.rglob("*") if path.is_file())
    shot_names = {path.name for path in media if path.name.startswith("shot-")}
    missing = sorted(EXPECTED_SHOTS - shot_names)
    unexpected = sorted(shot_names - EXPECTED_SHOTS)
    total_bytes = sum(path.stat().st_size for path in media)

    errors = []
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
