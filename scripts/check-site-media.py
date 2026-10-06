#!/usr/bin/env python3
"""Keep the tracked landing-page media set complete and intentionally small."""

from __future__ import annotations

import re
import sys
from html.parser import HTMLParser
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
MEDIA_DIR = REPO_ROOT / "site" / "img"
CAPTURE = REPO_ROOT / "e2e" / "shots" / "capture.cjs"
SHOTS_TABLE = re.compile(r"const SHOTS = \[(.*?)\];", re.S)
SHOT_ROW = re.compile(r'\[\d+, "([^"]+)", shot\d+\]')
SITE_PAGE = REPO_ROOT / "site" / "index.html"
PAGE_SHOT = re.compile(r"img/(shot-[^/]+\.webp)")
TEXT_ONLY_TAGS = {"textarea", "title"}
README = REPO_ROOT / "README.md"
README_SHOT = re.compile(r"/img/((?:[^\s)\"/]+/)*shot-[^\s)\"/]+\.webp)")
MEDIA_BUDGET_BYTES = 3 * 1024 * 1024 // 2


def registered_shots() -> set[str]:
    table = SHOTS_TABLE.search(CAPTURE.read_text(encoding="utf-8"))
    if table is None:
        return set()
    return set(SHOT_ROW.findall(table.group(1)))


class ShotSources(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.shots: list[str] = []
        self.text_only: str | None = None
        self.picture: list[str] | None = None
        self.picture_has_img = False
        self.templates = 0

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        if self.text_only:
            return
        if tag == "template":
            self.templates += 1
        if self.templates:
            return
        if tag in TEXT_ONLY_TAGS:
            self.text_only = tag
        if tag == "picture":
            self.picture = []
            self.picture_has_img = False
        if tag == "img":
            names = {"src", "srcset"}
        elif tag == "source" and self.picture is not None and not self.picture_has_img:
            names = {"srcset"}
        else:
            return
        image = self.picture if self.picture is not None else []
        for name, value in attrs:
            if name not in names:
                continue
            for url in attribute_urls(name, value or ""):
                shot = PAGE_SHOT.fullmatch(url)
                if shot and shot.group(1) not in image:
                    image.append(shot.group(1))
        if self.picture is None:
            self.shots.extend(image)
        elif tag == "img":
            self.picture_has_img = True

    def handle_endtag(self, tag: str) -> None:
        if self.text_only:
            if tag == self.text_only:
                self.text_only = None
            return
        if tag == "template" and self.templates:
            self.templates -= 1
        if self.templates:
            return
        if tag == "picture" and self.picture is not None:
            if self.picture_has_img:
                self.shots.extend(self.picture)
            self.picture = None


def attribute_urls(name: str, value: str) -> list[str]:
    if name == "src":
        return [value]
    if name == "srcset":
        return [candidate.split()[0] for candidate in value.split(",") if candidate.split()]
    return []


def referenced_shots() -> list[str]:
    parser = ShotSources()
    parser.feed(SITE_PAGE.read_text(encoding="utf-8"))
    parser.close()
    return parser.shots


def readme_shots() -> set[str]:
    return set(README_SHOT.findall(README.read_text(encoding="utf-8")))


def main() -> int:
    media = sorted(path for path in MEDIA_DIR.rglob("*") if path.is_file())
    shot_names = {
        path.relative_to(MEDIA_DIR).as_posix()
        for path in media
        if path.name.startswith("shot-")
    }
    expected = registered_shots()
    missing = sorted(expected - shot_names)
    unexpected = sorted(shot_names - expected)
    total_bytes = sum(path.stat().st_size for path in media)

    referenced = referenced_shots()
    repeated = sorted({name for name in referenced if referenced.count(name) > 1})
    readme = readme_shots()
    unregistered_in_readme = sorted(readme - expected)
    escaped = sorted(
        name for name in shot_names | expected | set(referenced) | readme if "%" in name
    )

    errors = []
    if escaped:
        errors.append(f"a screenshot path contains a percent escape: {', '.join(escaped)}")
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
