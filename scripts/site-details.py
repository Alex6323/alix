#!/usr/bin/env python3
"""site-details.py: inject personal legal/contact details into the built
site at deploy time, so they never enter git history.

The templates (site/impressum.html, site/datenschutz.html) carry placeholder
tokens marked as HTML comment pairs:

    <!--{{name}}-->[wird beim Deploy eingesetzt]<!--/{{name}}-->

Everything between the two comments (including the visible German text
"[wird beim Deploy eingesetzt]", "will be filled in at deploy time") is
replaced by the resolved value. That visible text is what a local preview
(opening the template straight from the checkout, no injection run) shows
instead of a raw, broken-looking `{{name}}` token.

Details JSON schema:

    {
      "name":   "Full legal name",
      "street": "Street and house number",
      "city":   "Postal code and city",
      "email":  "contact@alix.study",
      "phone":  ""
    }

`name`, `street`, `city`, `email` are required (non-empty). `phone` is
optional: when its value is "" (or the key is omitted entirely), any
template line wrapped in a `data-detail="phone"` element is deleted outright
rather than left showing an empty "Telefon:" label.

Modes:
  --inject SITE_DIR   Resolve every placeholder in SITE_DIR/impressum.html
                       and SITE_DIR/datenschutz.html and write the result
                       back in place. Aborts (no files written) if any
                       placeholder can't be resolved from the given details.
  --check SITE_DIR    Dry run: same resolution, no writes. Exits non-zero
                       and lists every unresolved placeholder / empty
                       required key if the details JSON wouldn't fully
                       resolve the templates.
  --sample             Print a sample details JSON to stdout and exit.

Details source: --details FILE reads the JSON from FILE (for local runs);
otherwise it's read from the SITE_DETAILS environment variable (what the
`pages` GitHub Actions workflow passes in from the SITE_DETAILS secret).

Idempotent: once a placeholder is resolved its comment markers are gone
from the text, so re-running --inject on an already-injected file changes
nothing.

No third-party dependencies: stdlib only, Python 3.

CI behavior: the `pages` workflow runs --check then --inject only when the
SITE_DETAILS secret is set. If it's absent (e.g. a contributor's fork, or
before the repo owner configures it), the workflow skips both steps and
deploys the templates as-is, showing the local-preview placeholder text.
It does not fail the build. A secret that IS set but fails --check (bad
JSON, missing/empty required key) fails the build loudly instead of
deploying broken or partial legal pages.
"""

from __future__ import annotations

import argparse
import html
import json
import os
import re
import sys
from pathlib import Path

TEMPLATE_FILES = ("impressum.html", "datenschutz.html")
REQUIRED_KEYS = ("name", "street", "city", "email")
OPTIONAL_KEYS = ("phone",)

SAMPLE_DETAILS = {
    "name": "Erika Musterfrau",
    "street": "Musterstraße 1",
    "city": "12345 Musterstadt",
    "email": "contact@alix.study",
    "phone": "",
}

# Matches a placeholder pair, e.g. <!--{{name}}-->...<!--/{{name}}-->.
# \1 backreferences the key so open/close must match.
MARKER_RE = re.compile(r"<!--\{\{(\w+)\}\}-->.*?<!--/\{\{\1\}\}-->", re.DOTALL)

# Matches a whole line carrying a data-detail="key" wrapper, for optional
# fields whose whole element gets dropped when the value is empty.
DETAIL_LINE_RE = re.compile(r'^[^\n]*data-detail="(\w+)"[^\n]*\n?', re.MULTILINE)


def find_marker_keys(text: str) -> set[str]:
    return {m.group(1) for m in MARKER_RE.finditer(text)}


def remove_empty_optional_lines(text: str, details: dict) -> str:
    def repl(m: re.Match) -> str:
        key = m.group(1)
        return m.group(0) if details.get(key, "") else ""

    return DETAIL_LINE_RE.sub(repl, text)


def apply_markers(text: str, details: dict) -> str:
    def repl(m: re.Match) -> str:
        key = m.group(1)
        if key in details:
            return html.escape(str(details[key]), quote=True)
        return m.group(0)  # key not provided at all: leave the marker as-is

    return MARKER_RE.sub(repl, text)


def process(text: str, details: dict) -> str:
    text = remove_empty_optional_lines(text, details)
    text = apply_markers(text, details)
    return text


def check_file(path: Path, details: dict) -> list[str]:
    text = path.read_text(encoding="utf-8")
    marker_keys = find_marker_keys(text)
    processed = process(text, details)
    unresolved = sorted(find_marker_keys(processed))
    problems = [
        f"{path.name}: placeholder {{{{{key}}}}} not resolved (missing from details JSON)"
        for key in unresolved
    ]
    for key in REQUIRED_KEYS:
        if key in marker_keys and not str(details.get(key, "")).strip():
            problems.append(f"{path.name}: required key '{key}' is empty")
    return problems


def load_details(details_file: str | None) -> dict | None:
    if details_file:
        try:
            raw = Path(details_file).read_text(encoding="utf-8")
        except OSError as e:
            print(f"error: cannot read {details_file}: {e}", file=sys.stderr)
            return None
    else:
        raw = os.environ.get("SITE_DETAILS", "")
        if not raw:
            print(
                "error: no details source: pass --details <file> or set SITE_DETAILS",
                file=sys.stderr,
            )
            return None
    try:
        data = json.loads(raw)
    except json.JSONDecodeError as e:
        print(f"error: details JSON is not valid: {e}", file=sys.stderr)
        return None
    if not isinstance(data, dict):
        print("error: details JSON must be an object", file=sys.stderr)
        return None
    return data


def collect_problems(site_dir: Path, details: dict) -> list[str]:
    problems = []
    for name in TEMPLATE_FILES:
        path = site_dir / name
        if not path.exists():
            problems.append(f"{name}: file not found in {site_dir}")
            continue
        problems.extend(check_file(path, details))
    return problems


def do_check(site_dir: Path, details: dict) -> int:
    problems = collect_problems(site_dir, details)
    if problems:
        print("site-details --check found problems:", file=sys.stderr)
        for p in problems:
            print(f"  - {p}", file=sys.stderr)
        return 1
    print(f"site-details --check: ok ({len(TEMPLATE_FILES)} files, all placeholders resolvable)")
    return 0


def do_inject(site_dir: Path, details: dict) -> int:
    problems = collect_problems(site_dir, details)
    if problems:
        print("site-details --inject aborted, problems found:", file=sys.stderr)
        for p in problems:
            print(f"  - {p}", file=sys.stderr)
        return 1
    for name in TEMPLATE_FILES:
        path = site_dir / name
        text = path.read_text(encoding="utf-8")
        new_text = process(text, details)
        tmp = path.with_name(path.name + ".tmp")
        tmp.write_text(new_text, encoding="utf-8")
        tmp.replace(path)
    print(f"site-details --inject: wrote {len(TEMPLATE_FILES)} file(s) in {site_dir}")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="site-details.py",
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--inject", metavar="SITE_DIR", help="resolve placeholders and write in place")
    mode.add_argument("--check", metavar="SITE_DIR", help="dry run: verify placeholders resolve")
    mode.add_argument("--sample", action="store_true", help="print a sample details JSON and exit")
    parser.add_argument(
        "--details",
        metavar="FILE",
        help="read details JSON from FILE instead of the SITE_DETAILS env var",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)

    if args.sample:
        print(json.dumps(SAMPLE_DETAILS, indent=2, ensure_ascii=False))
        return 0

    details = load_details(args.details)
    if details is None:
        return 1

    if args.check:
        return do_check(Path(args.check), details)
    return do_inject(Path(args.inject), details)


if __name__ == "__main__":
    sys.exit(main())
