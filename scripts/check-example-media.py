#!/usr/bin/env python3
"""Every example deck has an image, and every image belongs to an example.

This checks COMPLETENESS, not FRESHNESS. It cannot tell whether an image
still shows what alix renders today: that needs a browser and `cwebp`,
neither of which CI has. Only re-running `node e2e/shots/examples.cjs` and
finding no diff proves an image is current.

Read a green result here as "nothing is missing", never as "the images are
right".
"""

from __future__ import annotations

import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
EXAMPLES = REPO_ROOT / "docs" / "examples"
SETS = ("shapes", "syntax")
# Lossless WebP of one review screen sits well under this; the budget exists
# so a mistakenly committed PNG or an uncropped capture is caught here rather
# than in a reviewer's clone.
BUDGET_BYTES = 300 * 1024


def main() -> int:
    problems: list[str] = []
    total = 0

    for name in SETS:
        directory = EXAMPLES / name
        if not directory.is_dir():
            problems.append(f"missing example set: {directory.relative_to(REPO_ROOT)}")
            continue

        decks = {path.stem for path in directory.glob("*.md")}
        images = {path.stem for path in directory.glob("*.webp")}

        for deck in sorted(decks - images):
            problems.append(
                f"{name}/{deck}.md has no image; run `node e2e/shots/examples.cjs`"
            )
        for orphan in sorted(images - decks):
            problems.append(f"{name}/{orphan}.webp belongs to no example deck")

        for image in sorted(directory.glob("*.webp")):
            size = image.stat().st_size
            total += size
            if size > BUDGET_BYTES:
                problems.append(
                    f"{name}/{image.name} is {size // 1024} KiB, over the "
                    f"{BUDGET_BYTES // 1024} KiB budget"
                )

    if problems:
        for problem in problems:
            print(f"example-media: {problem}", file=sys.stderr)
        return 1

    print(f"example-media: every example has an image ({total // 1024} KiB total)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
