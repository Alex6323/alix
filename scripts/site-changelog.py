#!/usr/bin/env python3
"""site-changelog.py: render the "What's new" page (whatsnew.html) and the
landing-page teaser straight from CHANGELOG.md, at site-build time.

CHANGELOG.md is alix's single source of truth for user-facing history (see
CLAUDE.md: every user-facing change gets an entry). This script turns that
same file into a page, so the page can't drift from the changelog the way a
hand-maintained "news" page would.

Modes of use:

    site-changelog.py SITE_DIR

Reads <repo root>/CHANGELOG.md (or --changelog FILE), writes
SITE_DIR/whatsnew.html, and rewrites SITE_DIR/index.html in place, replacing
the content between its

    <!-- whatsnew-teaser:start --> ... <!-- whatsnew-teaser:end -->

markers with a short teaser (last release + its headline entries, plus an
honest "N changes in the works" count). The markers themselves are left in
place, so re-running is idempotent and a fresh checkout (which always carries
the static fallback teaser between the markers) renders sensibly even without
this script ever running.

Used two ways:
  - The `pages` GitHub Actions workflow runs it against the built `_site`
    artifact, before the site-details.py injection step (see pages.yml).
  - `make site` runs it against a scratch copy of site/ for local preview
    (never the tracked site/index.html itself, so a local preview never
    dirties the working tree).

Degrades, never fails the build, on a malformed CHANGELOG.md line: an empty
bullet, an empty version header, or a bullet found outside any `### ` section
is skipped (or leniently grouped) with a `::notice::` line to stdout (a GitHub
Actions log annotation; harmless plain text elsewhere) rather than raising. A
missing CHANGELOG.md or SITE_DIR is a real repo/workflow misconfiguration, not
a content hiccup, so those cases exit non-zero.

No third-party dependencies: stdlib only, Python 3.
"""

from __future__ import annotations

import argparse
import html
import re
import subprocess
import sys
from datetime import datetime
from pathlib import Path

# Hand-curated and deliberately capped at three: a longer "coming soon" list
# just rots as items ship and nobody prunes it, which reads as abandonment.
# Update this list by hand as real progress happens; nothing derives it.
UP_NEXT = [
    "The mobile app. The big one in progress: alix on your phone, built on "
    "the same core.",
    "Receiving a shared box inside the kids app.",
    "Smarter re-sharing: send an updated box without losing anyone's progress.",
]

TEASER_START = "<!-- whatsnew-teaser:start -->"
TEASER_END = "<!-- whatsnew-teaser:end -->"

VERSION_RE = re.compile(r"^## \[(?P<version>[^\]]*)\](?:\s*-\s*(?P<date>.+))?\s*$")
SECTION_RE = re.compile(r"^### (?P<name>.+?)\s*$")
BULLET_RE = re.compile(r"^- (?P<text>.*)$")

# Inline markdown actually used in CHANGELOG.md: **bold**, `code`, and
# [text](url) links (only seen in the header preamble today, but supported
# generically). Matched in priority order; everything else is escaped as
# plain text. Each match's captured text is escaped independently, so no raw
# changelog content reaches the page unescaped.
INLINE_RE = re.compile(
    r"\[(?P<link_text>[^\]]+)\]\((?P<link_url>[^)]+)\)"
    r"|\*\*(?P<bold>.+?)\*\*"
    r"|`(?P<code>[^`]+)`"
)


def esc(text: str) -> str:
    return html.escape(text, quote=False)


def render_inline(text: str) -> str:
    """Render one changelog line's inline markdown to safe HTML.

    Called per-bullet (never on a multi-bullet blob), so an unmatched marker
    (e.g. the lone ``` mention in the 0.4.0 note about fenced code blocks)
    just falls through to plain escaped text instead of pairing with an
    unrelated marker somewhere else in the document.
    """
    out = []
    pos = 0
    for m in INLINE_RE.finditer(text):
        out.append(esc(text[pos : m.start()]))
        if m.group("link_text") is not None:
            url = html.escape(m.group("link_url"), quote=True)
            out.append(f'<a href="{url}">{render_inline(m.group("link_text"))}</a>')
        elif m.group("bold") is not None:
            out.append(f"<strong>{render_inline(m.group('bold'))}</strong>")
        elif m.group("code") is not None:
            out.append(f"<code>{esc(m.group('code'))}</code>")
        pos = m.end()
    out.append(esc(text[pos:]))
    return "".join(out)


def repo_root() -> Path:
    return Path(__file__).resolve().parent.parent


def notice(message: str) -> None:
    print(f"::notice::site-changelog.py: {message}")


def parse_changelog(text: str) -> tuple[dict | None, list[dict]]:
    """Parse Keep-a-Changelog markdown into (unreleased, released[newest-first]).

    A release is {"version", "date", "intro", "sections": [(heading, [bullet, ...])]}.
    Section headings are kept verbatim (not restricted to Added/Changed/Fixed/
    Removed) so the freeform 0.1.0 section (Deck format, Review, ...) renders
    the same way canonical releases do.
    """
    unreleased: dict | None = None
    released: list[dict] = []
    current: dict | None = None
    current_bullets: list[str] | None = None
    in_intro = False

    for lineno, raw in enumerate(text.splitlines(), start=1):
        line = raw.rstrip()

        m = VERSION_RE.match(line)
        if m:
            version = m.group("version").strip()
            if not version:
                notice(f"line {lineno}: empty version header, skipped")
                continue
            current = {
                "version": version,
                "date": (m.group("date") or "").strip(),
                "intro": "",
                "sections": [],
            }
            current_bullets = None
            in_intro = True
            if version.lower() == "unreleased":
                unreleased = current
            else:
                released.append(current)
            continue

        m = SECTION_RE.match(line)
        if m and current is not None:
            bullets: list[str] = []
            current["sections"].append((m.group("name").strip(), bullets))
            current_bullets = bullets
            in_intro = False
            continue

        if current is None:
            continue  # preamble before the first "## [" header; not rendered

        m = BULLET_RE.match(line)
        if m:
            item = m.group("text").strip()
            if not item:
                notice(f"line {lineno}: empty bullet, skipped")
                continue
            if current_bullets is None:
                notice(f"line {lineno}: bullet outside any '### ' section, grouped under 'Notes'")
                current_bullets = []
                current["sections"].append(("Notes", current_bullets))
            current_bullets.append(item)
            in_intro = False
            continue

        if not line.strip():
            continue  # blank separator line, not meaningful on its own

        # Plain prose: either the release's intro paragraph (before its first
        # "### " heading) or the wrapped continuation of the current bullet.
        if in_intro:
            current["intro"] = (current["intro"] + " " + line.strip()).strip()
        elif current_bullets:
            current_bullets[-1] = (current_bullets[-1] + " " + line.strip()).strip()
        else:
            notice(f"line {lineno}: unrecognized line ignored: {line[:60]!r}")

    return unreleased, released


def count_bullets(release: dict | None) -> int:
    if not release:
        return 0
    return sum(len(bullets) for _, bullets in release["sections"])


def boldest_titles(release: dict, limit: int = 3) -> list[str]:
    """First `limit` bullets whose text opens with a bold lede, in document
    order (Added before Changed/Fixed, matching the changelog's own layout):
    these read as headline entries, the rest as detail."""
    titles = []
    for _, bullets in release["sections"]:
        for b in bullets:
            m = re.match(r"^\*\*(.+?)\*\*", b)
            if m:
                titles.append(m.group(1).rstrip("."))
                if len(titles) >= limit:
                    return titles
    return titles


def get_tag_dates(root: Path) -> list[tuple[str, str]]:
    """[(tag, "YYYY-MM-DD"), ...] ascending by date. Empty if git or tags are
    unavailable (e.g. a shallow checkout with no tags) rather than raising."""
    try:
        tags_out = subprocess.run(
            ["git", "tag", "--list", "v*"],
            cwd=root,
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    except (OSError, subprocess.CalledProcessError):
        return []
    dated = []
    for tag in (t for t in tags_out.splitlines() if t.strip()):
        try:
            date_out = subprocess.run(
                ["git", "log", "-1", "--format=%ad", "--date=short", tag],
                cwd=root,
                capture_output=True,
                text=True,
                check=True,
            ).stdout.strip()
        except (OSError, subprocess.CalledProcessError):
            continue
        if date_out:
            dated.append((tag, date_out))
    dated.sort(key=lambda td: td[1])
    return dated


def compute_cadence(root: Path) -> str | None:
    """A factual, computed cadence line from git tag dates (never from hand-
    typed changelog dates), e.g. "4 releases since June 2026, roughly one
    every 4 to 8 days." No future dates, no "expected" anything: the release
    policy is ships-when-ready (see RELEASING.md), so this only describes the
    past."""
    dated_tags = get_tag_dates(root)
    if not dated_tags:
        return None
    count = len(dated_tags)
    since = datetime.strptime(dated_tags[0][1], "%Y-%m-%d").strftime("%B %Y")
    if count == 1:
        return f"1 release so far, tagged {since}."

    gaps_days = []
    for (_, d1), (_, d2) in zip(dated_tags, dated_tags[1:]):
        delta = (datetime.strptime(d2, "%Y-%m-%d") - datetime.strptime(d1, "%Y-%m-%d")).days
        if delta > 0:
            gaps_days.append(delta)
    if not gaps_days:
        return f"{count} releases since {since}."

    min_days, max_days = min(gaps_days), max(gaps_days)
    if max_days < 14:
        lo, hi, unit = min_days, max_days, "day"
    else:
        lo = max(1, round(min_days / 7))
        hi = max(1, round(max_days / 7))
        unit = "week"
    span = f"{lo}" if lo == hi else f"{lo} to {hi}"
    unit = unit if hi == 1 and lo == hi else unit + "s"
    return f"{count} releases since {since}, roughly one every {span} {unit}."


def render_section_html(heading: str, bullets: list[str]) -> str:
    items = "\n".join(f"    <li>{render_inline(b)}</li>" for b in bullets)
    return f'  <h3>{esc(heading)}</h3>\n  <ul class="changes">\n{items}\n  </ul>'


def render_release_html(release: dict) -> str:
    date_html = f' <span class="date">{esc(release["date"])}</span>' if release["date"] else ""
    parts = ['<div class="release">', f'  <h2>v{esc(release["version"])}{date_html}</h2>']
    if release["intro"]:
        parts.append(f'  <p class="intro">{render_inline(release["intro"])}</p>')
    for heading, bullets in release["sections"]:
        if bullets:
            parts.append(render_section_html(heading, bullets))
    parts.append("</div>")
    return "\n".join(parts)


def render_unreleased_html(unreleased: dict | None) -> str:
    if not unreleased or count_bullets(unreleased) == 0:
        return '  <p class="empty">Nothing in progress right now.</p>'
    parts = []
    if unreleased["intro"]:
        parts.append(f'  <p class="intro">{render_inline(unreleased["intro"])}</p>')
    for heading, bullets in unreleased["sections"]:
        if bullets:
            parts.append(render_section_html(heading, bullets))
    return "\n".join(parts)


def render_up_next_html() -> str:
    items = "\n".join(f"    <li>{render_inline(line)}</li>" for line in UP_NEXT)
    return f'  <ul class="upnext-list">\n{items}\n  </ul>'


PAGE_CSS = """
  :root{
    --bg:#0d1117; --surface:#161b22; --line:#283039;
    --text:#e6edf3; --muted:#9aa4b2; --accent:#ffb454;
    --maxw:760px;
  }
  *{box-sizing:border-box}
  body{
    margin:0; background:var(--bg); color:var(--text);
    font:16px/1.6 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,Helvetica,Arial,sans-serif;
    -webkit-font-smoothing:antialiased;
  }
  a{color:var(--accent); text-decoration:none}
  a:hover{text-decoration:underline}
  code{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace; font-size:.92em;
    background:var(--surface); border:1px solid var(--line); border-radius:4px; padding:.05em .35em;
    overflow-wrap:anywhere}
  .wrap{max-width:var(--maxw); margin:0 auto; padding:0 24px}
  header{border-bottom:1px solid var(--line)}
  .nav{display:flex; align-items:center; height:64px; gap:22px}
  .brand{font-weight:800; font-size:19px; letter-spacing:.5px; color:var(--text)}
  .nav a{color:var(--muted); font-size:15px}
  .nav a:hover{color:var(--text); text-decoration:none}
  main{padding:52px 0 60px}
  h1{font-size:28px; margin:0 0 14px}
  h2{font-size:19px; margin:0 0 12px}
  h3{font-size:14px; margin:22px 0 6px; color:var(--muted); text-transform:uppercase; letter-spacing:.5px}
  .lede{color:var(--muted); margin:0 0 6px}
  .cadence{color:var(--muted); font-size:14px; margin:0 0 36px}
  section{margin:0 0 40px}
  .upnext{background:var(--surface); border:1px solid var(--line); border-radius:12px; padding:20px 24px; overflow-x:auto}
  .upnext h2{margin-bottom:10px}
  .upnext-list{margin:0; padding-left:20px}
  .upnext-list li{margin:0 0 6px}
  .inwork{overflow-x:auto}
  .inwork .hint{color:var(--muted); font-size:14px; margin:0 0 14px}
  .releases{overflow-x:auto}
  .release{padding:22px 0; border-top:1px solid var(--line)}
  .release:first-child{border-top:none}
  .release h2{display:flex; align-items:baseline; gap:10px}
  .release .date, .empty{color:var(--muted); font-size:14px; font-weight:400}
  .intro{color:var(--muted); margin:0 0 10px}
  ul.changes{margin:0 0 8px; padding-left:20px}
  ul.changes li{margin:0 0 8px}
  footer{border-top:1px solid var(--line); margin-top:64px; padding:34px 0; color:var(--muted); font-size:14px}
  .foot{display:flex; justify-content:space-between; flex-wrap:wrap; gap:14px}
  .foot a{color:var(--muted)}
  .foot a:hover{color:var(--text)}
"""


def render_whatsnew_page(unreleased: dict | None, released: list[dict], cadence: str | None) -> str:
    cadence_html = f'  <p class="cadence">{esc(cadence)}</p>' if cadence else ""
    releases_html = (
        "\n".join(render_release_html(r) for r in released)
        if released
        else '  <p class="empty">No tagged releases yet.</p>'
    )
    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>What's new: alix</title>
<meta name="description" content="What's shipped and what's in progress on alix, generated from the project's own changelog.">
<link rel="icon" href="/icon.svg" type="image/svg+xml">
<style>{PAGE_CSS}</style>
</head>
<body>
<header><div class="wrap nav">
  <div class="brand">alix</div>
  <a href="/">Home</a>
</div></header>

<main class="wrap">
  <h1>What&rsquo;s new</h1>
  <p class="lede">Generated from the project's own
    <a href="https://github.com/Alex6323/alix/blob/main/CHANGELOG.md">CHANGELOG.md</a>
    each time this site rebuilds, so it's only as current as the last push,
    with no separate page to remember to update.</p>
{cadence_html}

  <section class="upnext">
    <h2>Up next</h2>
{render_up_next_html()}
  </section>

  <section class="inwork">
    <h2>In the works</h2>
    <p class="hint">Merged on the main branch, not yet part of a tagged release.</p>
{render_unreleased_html(unreleased)}
  </section>

  <section class="releases">
{releases_html}
  </section>
</main>

<footer><div class="wrap foot">
  <div>MIT OR Apache-2.0 &middot; built by Alex with Claude</div>
  <div>
    <a href="/book/">Book</a> &middot;
    <a href="/slides.html">Slides</a> &middot;
    <a href="https://github.com/Alex6323/alix">GitHub</a> &middot;
    <a href="https://crates.io/crates/alix">crates.io</a> &middot;
    <a href="/impressum.html">Legal notice</a> &middot;
    <a href="/datenschutz.html">Privacy</a> &middot;
    <a href="mailto:contact@alix.study">contact@alix.study</a>
  </div>
</div></footer>
</body>
</html>
"""


def render_teaser_html(released: list[dict], unreleased: dict | None) -> str:
    if not released:
        return '<p class="whatsnew-line"><a href="/whatsnew.html">See what&rsquo;s new &rarr;</a></p>'
    last = released[0]
    titles = [render_inline(t) for t in boldest_titles(last)]
    bits = [f'<strong>v{esc(last["version"])}</strong>']
    if last["date"]:
        bits.append(f'({esc(last["date"])})')
    lead = " ".join(bits)
    if titles:
        lead += ": " + ", ".join(titles) + "."
    else:
        lead += "."
    unreleased_n = count_bullets(unreleased)
    tail = f" And in the works: {unreleased_n} change{'s' if unreleased_n != 1 else ''}." if unreleased_n else ""
    return f'<p class="whatsnew-line">{lead}{tail} <a href="/whatsnew.html">What&rsquo;s new &rarr;</a></p>'


def inject_teaser(text: str, teaser_html: str) -> str | None:
    start = text.find(TEASER_START)
    end = text.find(TEASER_END)
    if start == -1 or end == -1 or end < start:
        return None
    start_of_content = start + len(TEASER_START)
    return text[:start_of_content] + "\n  " + teaser_html + "\n  " + text[end:]


def write_atomic(path: Path, content: str) -> None:
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_text(content, encoding="utf-8")
    tmp.replace(path)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="site-changelog.py",
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "site_dir",
        metavar="SITE_DIR",
        help="directory to write whatsnew.html into and patch index.html's teaser markers in",
    )
    parser.add_argument(
        "--changelog",
        metavar="FILE",
        help="path to CHANGELOG.md (default: the repo's own CHANGELOG.md next to this script)",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    root = repo_root()
    changelog_path = Path(args.changelog) if args.changelog else root / "CHANGELOG.md"
    site_dir = Path(args.site_dir)

    if not changelog_path.exists():
        print(f"error: changelog not found at {changelog_path}", file=sys.stderr)
        return 1
    if not site_dir.is_dir():
        print(f"error: site dir not found: {site_dir}", file=sys.stderr)
        return 1

    text = changelog_path.read_text(encoding="utf-8")
    unreleased, released = parse_changelog(text)
    cadence = compute_cadence(root)

    write_atomic(site_dir / "whatsnew.html", render_whatsnew_page(unreleased, released, cadence))

    index_path = site_dir / "index.html"
    if index_path.exists():
        teaser_html = render_teaser_html(released, unreleased)
        index_text = index_path.read_text(encoding="utf-8")
        new_index_text = inject_teaser(index_text, teaser_html)
        if new_index_text is None:
            notice(f"teaser markers not found in {index_path}, left unchanged")
        else:
            write_atomic(index_path, new_index_text)
    else:
        notice(f"{index_path} not found, skipping the landing-page teaser")

    print(
        f"site-changelog.py: wrote whatsnew.html "
        f"({len(released)} release(s), {count_bullets(unreleased)} unreleased entries)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
