#!/usr/bin/env python3
"""site-changelog.py: render the "What's new" page (whatsnew.html) and the
landing-page teaser straight from CHANGELOG.md, at site-build time.

CHANGELOG.md is alix's single source of truth for user-facing history (see
CLAUDE.md: every user-facing change gets an entry). This script turns that
same file into a page, so the page can't drift from the changelog the way a
hand-maintained "news" page would.

The page leads with an interactive timeline strip: release dots (from git
tags), small entry dots for each commit that landed changelog bullets (with
a hover/focus popover listing what landed), and three pulsing "up next" dots
at the open right end. Per-entry timestamps come from `git log -p` over
CHANGELOG.md: each current bullet is attributed to the commit that first
added its line. Below the strip the text stays the honest record: the
"Up next" and "In the works" lists, then each release collapsed into a
<details> block.

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
    The workflow checks out with fetch-depth: 0 on purpose: both the tag
    dates (cadence line, release dots) and the per-entry attribution need
    real history, and a default shallow clone has neither.
  - `make site` runs it against a scratch copy of site/ for local preview
    (never the tracked site/index.html itself, so a local preview never
    dirties the working tree).

Degrades, never fails the build, on odd input: a malformed CHANGELOG.md line
(empty bullet, empty version header, a bullet outside any `### ` section) is
skipped or leniently grouped with a `::notice::` line to stdout (a GitHub
Actions log annotation; harmless plain text elsewhere); a bullet whose
addition commit can't be found in the git history (edited or reflowed lines)
falls back to its section's release date, or for Unreleased to the newest
changelog commit; no git history at all just omits the timeline strip. A
missing CHANGELOG.md or SITE_DIR is a real repo/workflow misconfiguration,
not a content hiccup, so those cases exit non-zero.

No third-party dependencies: stdlib only, Python 3.
"""

from __future__ import annotations

import argparse
import html
import re
import subprocess
import sys
from datetime import date as date_type
from datetime import datetime
from pathlib import Path

# Hand-curated and deliberately capped at three: a longer "coming soon" list
# just rots as items ship and nobody prunes it, which reads as abandonment.
# Update this list by hand as real progress happens; nothing derives it.
# Shown twice, deliberately: as the pulsing dots at the timeline's open right
# end, and as the plain-text "Up next" list below (the no-JS record).
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

# ---------------------------------------------------------------------------
# Timeline layout constants (px). Time-proportional spacing with clamps: a
# dense day can't overlap dots, a quiet week can't waste meters.
PX_PER_DAY = 14
MIN_GAP = 20
MAX_GAP = 120
PAD_X = 70  # canvas end padding, so edge labels never clip
UP_GAP = 76  # gap between the last real dot and the "up next" trio
UP_SPACING = 46  # spacing inside the trio


def esc(text: str) -> str:
    return html.escape(text, quote=False)


def render_inline(text: str) -> str:
    """Render one changelog line's inline markdown to safe HTML.

    Called per-bullet (never on a multi-bullet blob), so an unmatched marker
    (e.g. a lone ``` mention in prose) just falls through to plain escaped
    text instead of pairing with an unrelated marker somewhere else in the
    document.
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


def strip_inline(text: str) -> str:
    """Reduce a bullet's inline markdown to plain text (for popovers and
    aria labels, where markup would just be noise)."""
    text = re.sub(r"\[([^\]]+)\]\([^)]+\)", r"\1", text)
    return text.replace("**", "").replace("`", "")


def truncate(text: str, limit: int = 170) -> str:
    if len(text) <= limit:
        return text
    cut = text[:limit].rsplit(" ", 1)[0]
    return cut.rstrip(" ,.;:") + "…"


def display_version(version: str) -> str:
    """"0.4.0" renders as "v0.4.0"; a non-numeric name like "flash 0.1.0"
    stays as written."""
    return f"v{version}" if version[:1].isdigit() else version


def short_date(iso: str) -> str:
    """"2026-07-11" -> "Jul 11" (for strip labels; the full date stays in
    the text sections)."""
    try:
        dt = datetime.strptime(iso, "%Y-%m-%d")
    except ValueError:
        return iso
    return f"{dt.strftime('%b')} {dt.day}"


def repo_root() -> Path:
    return Path(__file__).resolve().parent.parent


def notice(message: str) -> None:
    print(f"::notice::site-changelog.py: {message}")


def normalize(line: str) -> str:
    return " ".join(line.split())


# ---------------------------------------------------------------------------
# Changelog parsing


def parse_changelog(text: str) -> tuple[dict | None, list[dict]]:
    """Parse Keep-a-Changelog markdown into (unreleased, released[newest-first]).

    A release is {"version", "date", "intro", "sections": [(heading, [bullet, ...])]}
    where each bullet is {"text": joined text, "first": its raw first physical
    line} (the first line is what the git attribution matches against).
    Section headings are kept verbatim (not restricted to Added/Changed/Fixed/
    Removed) so the freeform first-release section (Deck format, Review, ...)
    renders the same way canonical releases do.
    """
    unreleased: dict | None = None
    released: list[dict] = []
    current: dict | None = None
    current_bullets: list[dict] | None = None
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
            bullets: list[dict] = []
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
            current_bullets.append({"text": item, "first": line})
            in_intro = False
            continue

        if not line.strip():
            continue  # blank separator line, not meaningful on its own

        # Plain prose: either the release's intro paragraph (before its first
        # "### " heading) or the wrapped continuation of the current bullet.
        if in_intro:
            current["intro"] = (current["intro"] + " " + line.strip()).strip()
        elif current_bullets:
            current_bullets[-1]["text"] = (current_bullets[-1]["text"] + " " + line.strip()).strip()
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
            m = re.match(r"^\*\*(.+?)\*\*", b["text"])
            if m:
                titles.append(m.group(1).rstrip("."))
                if len(titles) >= limit:
                    return titles
    return titles


# ---------------------------------------------------------------------------
# Git history: tag dates (cadence + release dots) and per-bullet attribution


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


def changelog_addition_history(root: Path) -> tuple[dict, int]:
    """Walk `git log -p --reverse -- CHANGELOG.md` and map each added bullet
    first-line (normalized) to the (hash, unix ts, short date) of the earliest
    commit that added it. Also returns the newest changelog commit's ts (the
    Unreleased fallback). Empty map on any git failure (shallow clone, no
    repo): the caller then skips the timeline rather than failing the build.
    """
    try:
        out = subprocess.run(
            [
                "git",
                "log",
                "--reverse",
                "-p",
                "--format=COMMIT %H %ct %ad",
                "--date=short",
                "--",
                "CHANGELOG.md",
            ],
            cwd=root,
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    except (OSError, subprocess.CalledProcessError):
        return {}, 0

    commit_re = re.compile(r"^COMMIT ([0-9a-f]{40}) (\d+) (\d{4}-\d{2}-\d{2})$")
    added: dict[str, tuple[str, int, str]] = {}
    current: tuple[str, int, str] | None = None
    newest_ts = 0
    for line in out.splitlines():
        m = commit_re.match(line)
        if m:
            current = (m.group(1), int(m.group(2)), m.group(3))
            newest_ts = max(newest_ts, int(m.group(2)))
            continue
        if current and line.startswith("+- "):
            key = normalize(line[1:])
            if key not in added:  # --reverse: first occurrence = earliest
                added[key] = current
    return added, newest_ts


def noon_ts(iso: str) -> int | None:
    try:
        dt = datetime.strptime(iso, "%Y-%m-%d")
    except ValueError:
        return None
    return int(dt.replace(hour=12).timestamp())


def attribute_bullets(unreleased: dict | None, released: list[dict], root: Path) -> list[dict]:
    """Group every current bullet by the commit that landed it, for the
    timeline's entry dots: one event per landing commit, [{ts, date, items}].

    A bullet whose addition commit can't be found (its line was edited or
    reflowed since) falls back to its section's release date, or for
    Unreleased to the newest changelog commit; the misses are reported as one
    aggregate notice, never a failure.
    """
    added, newest_ts = changelog_addition_history(root)
    if not added:
        return []

    events: dict[str, dict] = {}
    fallbacks = 0

    def record(key: str, ts: int, date: str, text: str) -> None:
        ev = events.setdefault(key, {"ts": ts, "date": date, "items": []})
        ev["items"].append(text)

    sections: list[tuple[dict, bool]] = []
    if unreleased:
        sections.append((unreleased, True))
    sections.extend((rel, False) for rel in released)

    for rel, is_unreleased in sections:
        rel_ts = noon_ts(rel["date"]) if rel["date"] else None
        for _, bullets in rel["sections"]:
            for b in bullets:
                key = normalize(b["first"])
                hit = added.get(key)
                if hit is None:
                    # Tolerant second tier: the line was reflowed but its
                    # opening words survived somewhere in an added line.
                    prefix = key[:60]
                    if len(prefix) >= 20:
                        for k, v in added.items():
                            if k.startswith(prefix) or key.startswith(k[:60]):
                                hit = v
                                break
                if hit is not None:
                    commit, ts, date = hit
                    record(commit, ts, date, b["text"])
                elif is_unreleased and newest_ts:
                    fallbacks += 1
                    record(
                        "fallback-unreleased",
                        newest_ts,
                        datetime.fromtimestamp(newest_ts).strftime("%Y-%m-%d"),
                        b["text"],
                    )
                elif rel_ts is not None:
                    fallbacks += 1
                    record(f"fallback-{rel['version']}", rel_ts, rel["date"], b["text"])
                else:
                    fallbacks += 1  # no commit, no date: drop from the strip only

    if fallbacks:
        notice(f"{fallbacks} entries fell back to a section date (no addition commit found)")
    return sorted(events.values(), key=lambda e: e["ts"])


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


# ---------------------------------------------------------------------------
# Timeline strip


def month_ticks(nodes: list[dict]) -> list[dict]:
    """Sparse orientation ticks: one at the first dot (month + year), then one
    per first-of-month inside the range, positioned by piecewise-linear
    interpolation between the surrounding dots (the strip's spacing is
    clamped, so time-to-x isn't globally linear)."""
    if not nodes:
        return []
    ticks = []
    first_dt = datetime.fromtimestamp(nodes[0]["ts"])
    ticks.append({"x": nodes[0]["x"], "label": first_dt.strftime("%b %Y")})

    # Walk month boundaries strictly after the first node.
    year, month = first_dt.year, first_dt.month
    last_ts = nodes[-1]["ts"]
    while True:
        month += 1
        if month > 12:
            month, year = 1, year + 1
        boundary = int(datetime(year, month, 1).timestamp())
        if boundary > last_ts:
            break
        for a, b in zip(nodes, nodes[1:]):
            if a["ts"] <= boundary <= b["ts"] and b["ts"] > a["ts"]:
                frac = (boundary - a["ts"]) / (b["ts"] - a["ts"])
                x = a["x"] + frac * (b["x"] - a["x"])
                fmt = "%b %Y" if month == 1 else "%b"
                ticks.append({"x": round(x), "label": date_type(year, month, 1).strftime(fmt)})
                break
    return ticks


def layout_timeline(entry_events: list[dict], tags: list[tuple[str, str]]) -> dict | None:
    """Position every dot on the strip. Returns {nodes, ticks, up_xs, width}
    or None when there's nothing to draw."""
    nodes: list[dict] = []
    for ev in entry_events:
        nodes.append({"kind": "entry", "ts": ev["ts"], "date": ev["date"], "items": ev["items"]})
    for tag, date in tags:
        ts = noon_ts(date)
        if ts is not None:
            nodes.append({"kind": "release", "ts": ts, "date": date, "version": tag})
    if not nodes:
        return None

    # Entries sort before a same-moment release: the release dot closes what
    # landed before it.
    nodes.sort(key=lambda n: (n["ts"], 0 if n["kind"] == "entry" else 1))

    x = float(PAD_X)
    prev_ts: int | None = None
    for n in nodes:
        if prev_ts is not None:
            gap = (n["ts"] - prev_ts) / 86400 * PX_PER_DAY
            x += min(max(gap, MIN_GAP), MAX_GAP)
        n["x"] = round(x)
        prev_ts = n["ts"]

    up_start = nodes[-1]["x"] + UP_GAP
    up_xs = [up_start + i * UP_SPACING for i in range(len(UP_NEXT))]
    width = (up_xs[-1] if up_xs else nodes[-1]["x"]) + PAD_X
    return {"nodes": nodes, "ticks": month_ticks(nodes), "up_xs": up_xs, "width": width}


def render_timeline_html(layout: dict) -> str:
    """The strip: an axis with month ticks, entry/release/upcoming dots, the
    hidden per-dot popover sources, and the shared popover element. All text
    the popovers show also lives in the sections below (they enhance, never
    gate)."""
    dots: list[str] = []
    pops: list[str] = []

    for i, n in enumerate(layout["nodes"]):
        if n["kind"] == "release":
            pid = f"tlr{i}"
            label = esc(n["version"])
            dots.append(
                f'<button type="button" class="tl-dot release" style="left:{n["x"]}px"'
                f' data-pop="{pid}" aria-expanded="false"'
                f' aria-label="Release {esc(n["version"])}, {esc(n["date"])}">'
                f'<span class="d"></span>'
                f'<span class="tl-rlabel">{label} <span class="dt">&middot; {esc(short_date(n["date"]))}</span></span>'
                f"</button>"
            )
            pops.append(
                f'<div data-for="{pid}"><h4>{label} &middot; {esc(n["date"])}</h4>'
                f"<p>Tagged release. Full notes below.</p></div>"
            )
        else:
            pid = f"tle{i}"
            count = len(n["items"])
            word = "change" if count == 1 else "changes"
            items = "".join(f"<li>{esc(truncate(strip_inline(t)))}</li>" for t in n["items"])
            dots.append(
                f'<button type="button" class="tl-dot entry" style="left:{n["x"]}px"'
                f' data-pop="{pid}" aria-expanded="false"'
                f' aria-label="{count} {word}, {esc(short_date(n["date"]))}">'
                f'<span class="d"></span></button>'
            )
            pops.append(
                f'<div data-for="{pid}"><h4>{esc(short_date(n["date"]))} &middot; {count} {word}</h4>'
                f"<ul>{items}</ul></div>"
            )

    for i, (x, text) in enumerate(zip(layout["up_xs"], UP_NEXT)):
        pid = f"tlu{i}"
        dots.append(
            f'<button type="button" class="tl-dot upcoming" style="left:{x}px"'
            f' data-pop="{pid}" aria-expanded="false"'
            f' aria-label="Up next: {esc(truncate(strip_inline(text), 80))}">'
            f'<span class="d"></span></button>'
        )
        pops.append(f'<div data-for="{pid}"><h4>Up next</h4><p>{esc(text)}</p></div>')

    up_label = ""
    if layout["up_xs"]:
        mid = layout["up_xs"][len(layout["up_xs"]) // 2]
        up_label = f'<span class="tl-up-label" style="left:{mid}px">up next</span>'

    ticks = "".join(
        f'<span class="tl-tick" style="left:{t["x"]}px"></span>'
        f'<span class="tl-tick-label" style="left:{t["x"]}px">{esc(t["label"])}</span>'
        for t in layout["ticks"]
    )

    return f"""  <section class="timeline" aria-label="Development timeline">
    <div class="tl-scroll">
      <div class="tl-canvas" style="width:{layout["width"]}px">
        <span class="tl-axis"></span>
        {ticks}
        {"".join(dots)}
        {up_label}
      </div>
    </div>
    <p class="tl-hint">Every dot is work that landed; hover or tap one. Newest on the right.</p>
    <div id="tl-pop-srcs" hidden>{"".join(pops)}</div>
    <div id="tl-pop" role="tooltip" hidden></div>
  </section>"""


# The popover + scroll behavior. Inline and dependency-free; everything it
# reveals is also plain text further down the page, so no-JS loses nothing.
TIMELINE_JS = """
(function () {
  var scroll = document.querySelector('.tl-scroll');
  if (scroll) scroll.scrollLeft = scroll.scrollWidth; // newest (right end) first
  var pop = document.getElementById('tl-pop');
  var srcs = document.getElementById('tl-pop-srcs');
  if (!pop || !srcs) return;
  var current = null;

  function position(btn) {
    var r = btn.getBoundingClientRect();
    pop.style.left = '0px'; pop.style.top = '0px'; // reset before measuring
    var w = pop.offsetWidth, h = pop.offsetHeight;
    var left = Math.min(Math.max(r.left + r.width / 2 - w / 2, 8), window.innerWidth - w - 8);
    var top = r.top - h - 10;
    if (top < 8) top = r.bottom + 10; // flip below when there's no room above
    pop.style.left = left + 'px';
    pop.style.top = top + 'px';
  }
  function open(btn) {
    var src = srcs.querySelector('[data-for="' + btn.dataset.pop + '"]');
    if (!src) return;
    if (current && current !== btn) current.setAttribute('aria-expanded', 'false');
    pop.innerHTML = src.innerHTML; // build-time generated + escaped content
    pop.hidden = false;
    position(btn);
    btn.setAttribute('aria-expanded', 'true');
    current = btn;
  }
  function close() {
    if (!current) return;
    current.setAttribute('aria-expanded', 'false');
    pop.hidden = true;
    current = null;
  }
  document.querySelectorAll('.tl-dot').forEach(function (d) {
    d.addEventListener('mouseenter', function () { open(d); });
    d.addEventListener('mouseleave', function () {
      if (document.activeElement !== d) close();
    });
    d.addEventListener('focus', function () { open(d); });
    d.addEventListener('blur', close);
    d.addEventListener('click', function (e) { e.stopPropagation(); open(d); });
  });
  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape') close();
  });
  document.addEventListener('click', function (e) {
    if (!e.target.closest || !e.target.closest('.tl-dot')) close();
  });
  // Keep the popover glued to its dot while the strip (or page) scrolls.
  ['scroll', 'resize'].forEach(function (evt) {
    window.addEventListener(evt, function () {
      if (current) position(current);
    }, true);
  });
})();
"""


# ---------------------------------------------------------------------------
# Page rendering


def render_section_html(heading: str, bullets: list[dict]) -> str:
    items = "\n".join(f"    <li>{render_inline(b['text'])}</li>" for b in bullets)
    return f'  <h3>{esc(heading)}</h3>\n  <ul class="changes">\n{items}\n  </ul>'


def render_release_html(release: dict) -> str:
    n = count_bullets(release)
    word = "change" if n == 1 else "changes"
    date_meta = f"{esc(release['date'])} &middot; " if release["date"] else ""
    parts = [
        '<details class="release">',
        f"  <summary>{esc(display_version(release['version']))}"
        f' <span class="meta">{date_meta}{n} {word}</span></summary>',
    ]
    if release["intro"]:
        parts.append(f'  <p class="intro">{render_inline(release["intro"])}</p>')
    for heading, bullets in release["sections"]:
        if bullets:
            parts.append(render_section_html(heading, bullets))
    parts.append("</details>")
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
    --tl-entry:#6e7a8a; /* entry-dot gray: >=3:1 on --bg, deliberately quiet */
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
  .cadence{color:var(--muted); font-size:14px; margin:0 0 30px}
  section{margin:0 0 40px}

  /* The timeline strip. Only .tl-scroll scrolls sideways; the page never does. */
  .timeline{margin:0 0 44px}
  .tl-scroll{overflow-x:auto; -webkit-overflow-scrolling:touch;
    border:1px solid var(--line); border-radius:12px; background:var(--surface)}
  .tl-canvas{position:relative; height:150px}
  .tl-axis{position:absolute; left:0; right:0; top:86px; border-top:1px solid var(--line)}
  .tl-tick{position:absolute; top:86px; width:1px; height:7px; background:var(--line)}
  .tl-tick-label{position:absolute; top:97px; transform:translateX(-50%);
    color:var(--muted); font-size:12px; white-space:nowrap}
  .tl-dot{position:absolute; top:86px; transform:translate(-50%,-50%);
    width:28px; height:28px; /* hit target well past the painted dot */
    display:flex; align-items:center; justify-content:center;
    background:none; border:none; padding:0; margin:0; cursor:pointer}
  .tl-dot .d{display:block; border-radius:50%;
    box-shadow:0 0 0 2px var(--bg)} /* surface ring: legible when dots crowd */
  .tl-dot.entry .d{width:8px; height:8px; background:var(--tl-entry)}
  .tl-dot.release .d{width:14px; height:14px; background:var(--accent)}
  .tl-dot.upcoming .d{width:12px; height:12px; background:transparent;
    border:2px dashed var(--accent);
    animation:tl-pulse 2.6s ease-in-out infinite}
  @keyframes tl-pulse{
    0%,100%{box-shadow:0 0 0 2px var(--bg), 0 0 0 0 rgba(255,180,84,0)}
    50%{box-shadow:0 0 0 2px var(--bg), 0 0 12px 3px rgba(255,180,84,.35)}
  }
  @media (prefers-reduced-motion: reduce){
    .tl-dot.upcoming .d{animation:none}
  }
  .tl-dot:focus-visible{outline:none}
  .tl-dot:focus-visible .d{outline:2px solid var(--accent); outline-offset:3px}
  .tl-dot:hover .d{filter:brightness(1.15)}
  .tl-rlabel{position:absolute; bottom:32px; left:50%; transform:translateX(-50%);
    white-space:nowrap; font-size:12.5px; font-weight:600; color:var(--text)}
  .tl-rlabel .dt{color:var(--muted); font-weight:400}
  .tl-up-label{position:absolute; top:36px; transform:translateX(-50%);
    color:var(--muted); font-size:12px; white-space:nowrap}
  .tl-hint{color:var(--muted); font-size:13px; margin:10px 2px 0}
  #tl-pop{position:fixed; z-index:10; max-width:340px; max-height:45vh; overflow-y:auto;
    background:var(--surface); border:1px solid var(--line); border-radius:10px;
    padding:12px 14px; font-size:13.5px; line-height:1.5;
    box-shadow:0 12px 32px rgba(0,0,0,.5)}
  #tl-pop h4{margin:0 0 6px; font-size:13px; color:var(--text)}
  #tl-pop p{margin:0; color:var(--muted)}
  #tl-pop ul{margin:0; padding-left:16px; color:var(--muted)}
  #tl-pop li{margin:0 0 5px}

  .upnext{background:var(--surface); border:1px solid var(--line); border-radius:12px;
    padding:16px 22px; overflow-x:auto}
  .upnext h2{margin-bottom:8px; font-size:16px}
  .upnext-list{margin:0; padding-left:20px; font-size:14.5px}
  .upnext-list li{margin:0 0 5px}
  .inwork{overflow-x:auto; font-size:14.5px}
  .inwork h2{font-size:19px}
  .inwork .hint{color:var(--muted); font-size:14px; margin:0 0 10px}
  .releases{overflow-x:auto; font-size:14.5px}
  details.release{border-top:1px solid var(--line); padding:14px 0}
  details.release summary{cursor:pointer; font-size:16.5px; font-weight:600;
    padding:2px 0}
  details.release summary:hover{color:var(--accent)}
  details.release summary .meta{color:var(--muted); font-weight:400; font-size:14px}
  .intro{color:var(--muted); margin:10px 0}
  ul.changes{margin:0 0 8px; padding-left:20px}
  ul.changes li{margin:0 0 8px}
  .empty{color:var(--muted); font-size:14px}
  footer{border-top:1px solid var(--line); margin-top:64px; padding:34px 0; color:var(--muted); font-size:14px}
  .foot{display:flex; justify-content:space-between; flex-wrap:wrap; gap:14px}
  .foot a{color:var(--muted)}
  .foot a:hover{color:var(--text)}
"""


def render_whatsnew_page(
    unreleased: dict | None,
    released: list[dict],
    cadence: str | None,
    timeline_html: str,
) -> str:
    cadence_html = f'  <p class="cadence">{esc(cadence)}</p>' if cadence else ""
    releases_html = (
        "\n".join(render_release_html(r) for r in released)
        if released
        else '  <p class="empty">No tagged releases yet.</p>'
    )
    timeline_block = f"\n{timeline_html}\n" if timeline_html else ""
    timeline_script = f"<script>{TIMELINE_JS}</script>\n" if timeline_html else ""
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
{timeline_block}
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
{timeline_script}</body>
</html>
"""


def render_teaser_html(released: list[dict], unreleased: dict | None) -> str:
    if not released:
        return '<p class="whatsnew-line"><a href="/whatsnew.html">See what&rsquo;s new &rarr;</a></p>'
    last = released[0]
    titles = [render_inline(t) for t in boldest_titles(last)]
    bits = [f"<strong>{esc(display_version(last['version']))}</strong>"]
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

    entry_events = attribute_bullets(unreleased, released, root)
    layout = layout_timeline(entry_events, get_tag_dates(root))
    if layout is None:
        notice("no git history available, rendering without the timeline strip")
        timeline_html = ""
    else:
        timeline_html = render_timeline_html(layout)

    write_atomic(
        site_dir / "whatsnew.html",
        render_whatsnew_page(unreleased, released, cadence, timeline_html),
    )

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

    dots = len(layout["nodes"]) if layout else 0
    print(
        f"site-changelog.py: wrote whatsnew.html "
        f"({len(released)} release(s), {count_bullets(unreleased)} unreleased entries, "
        f"{dots} timeline dots)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
