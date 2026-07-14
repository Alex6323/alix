#!/usr/bin/env python3
"""stats.py: print alix's download and visitor numbers.

Downloads come from GitHub releases and crates.io (public APIs, no auth).
Visitors come from GoatCounter when the GOATCOUNTER_TOKEN environment
variable is set (create a read-statistics token at alix.goatcounter.com
under Settings, API); without it that section is skipped with a hint.
Stdlib only (urllib.request, json) so it runs anywhere `make stats` runs.
Sections are independent; a failure in one (rate limit, no network) prints
a clear one-line message and still lets the others run.
"""

from __future__ import annotations

import datetime
import json
import os
import sys
import urllib.error
import urllib.request

GITHUB_RELEASES_URL = "https://api.github.com/repos/Alex6323/alix/releases?per_page=100"
CRATE_URL = "https://crates.io/api/v1/crates/alix"
CRATE_DOWNLOADS_URL = "https://crates.io/api/v1/crates/alix/downloads"

GITHUB_USER_AGENT = "alix-stats-script (https://github.com/Alex6323/alix)"
CRATES_USER_AGENT = "alix-stats-script (contact: contact@alix.study)"

# alix-<target-triple>.(tar.gz|zip) -> a short column label. Anything not
# listed here falls back to the triple itself, so a new build target still
# shows up (just wider) rather than being dropped.
PLATFORM_LABELS = {
    "aarch64-apple-darwin": "mac-arm",
    "x86_64-apple-darwin": "mac-x64",
    "x86_64-unknown-linux-gnu": "linux",
    "x86_64-pc-windows-msvc": "windows",
}
PLATFORM_ORDER = ["mac-arm", "mac-x64", "linux", "windows"]


def fetch_json(
    url: str, user_agent: str, extra_headers: dict[str, str] | None = None
) -> tuple[object | None, str | None]:
    """GET url as JSON. Returns (data, None) or (None, error message)."""
    headers = {"User-Agent": user_agent, "Accept": "application/json"}
    if extra_headers:
        headers.update(extra_headers)
    request = urllib.request.Request(url, headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=10) as response:
            return json.loads(response.read().decode("utf-8")), None
    except urllib.error.HTTPError as e:
        if e.code == 403 and e.headers.get("X-RateLimit-Remaining") == "0":
            return None, f"{url}: rate limited (GitHub allows 60 unauthenticated requests/hour)"
        return None, f"{url}: HTTP {e.code} {e.reason}"
    except urllib.error.URLError as e:
        return None, f"{url}: network error ({e.reason})"
    except (json.JSONDecodeError, TimeoutError) as e:
        return None, f"{url}: {e}"


def platform_label(asset_name: str) -> str:
    stem = asset_name
    for prefix in ("alix-",):
        if stem.startswith(prefix):
            stem = stem[len(prefix) :]
    for suffix in (".tar.gz", ".zip"):
        if stem.endswith(suffix):
            stem = stem[: -len(suffix)]
    return PLATFORM_LABELS.get(stem, stem)


def bold(text: str) -> str:
    return f"\033[1m{text}\033[0m" if sys.stdout.isatty() else text


def print_table(headers: list[str], rows: list[list[str]]) -> None:
    widths = [len(h) for h in headers]
    for row in rows:
        for i, cell in enumerate(row):
            widths[i] = max(widths[i], len(cell))
    line = "  ".join(h.rjust(widths[i]) if i > 0 else h.ljust(widths[i]) for i, h in enumerate(headers))
    print(line)
    print("-" * len(line))
    for row in rows:
        print("  ".join(cell.rjust(widths[i]) if i > 0 else cell.ljust(widths[i]) for i, cell in enumerate(row)))


def github_section() -> int:
    print(bold("GitHub releases") + "  (github.com/Alex6323/alix)")
    releases, error = fetch_json(GITHUB_RELEASES_URL, GITHUB_USER_AGENT)
    if error:
        print(f"  error: {error}")
        return 0
    if not releases:
        print("  no releases found")
        return 0

    # Column order: platforms seen, in our preferred order first, then any
    # unrecognized ones (a new build target) in first-seen order.
    seen_platforms: list[str] = []
    per_release: list[tuple[str, dict[str, int]]] = []
    for release in releases:
        tag = release.get("tag_name", "?")
        counts: dict[str, int] = {}
        for asset in release.get("assets", []):
            label = platform_label(asset.get("name", ""))
            counts[label] = counts.get(label, 0) + asset.get("download_count", 0)
            if label not in seen_platforms:
                seen_platforms.append(label)
        per_release.append((tag, counts))

    ordered_platforms = [p for p in PLATFORM_ORDER if p in seen_platforms]
    ordered_platforms += [p for p in seen_platforms if p not in ordered_platforms]

    if not ordered_platforms:
        print("  no release assets found (releases with no uploaded binaries)")
        return 0

    headers = ["Tag"] + ordered_platforms + ["Total"]
    rows = []
    totals = {p: 0 for p in ordered_platforms}
    grand_total = 0
    for tag, counts in per_release:
        row_total = sum(counts.values())
        grand_total += row_total
        row = [tag]
        for p in ordered_platforms:
            # A release with zero uploaded assets shows "-" (no data) rather
            # than "0" (a real, counted download total of zero).
            row.append("-" if not counts else str(counts.get(p, 0)))
            totals[p] += counts.get(p, 0)
        row.append(str(row_total))
        rows.append(row)
    rows.append(["Total"] + [str(totals[p]) for p in ordered_platforms] + [str(grand_total)])

    print_table(headers, rows)
    return grand_total


def crates_section() -> tuple[int, int]:
    print()
    print(bold("crates.io") + "  (crates.io/crates/alix)")
    data, error = fetch_json(CRATE_URL, CRATES_USER_AGENT)
    if error:
        print(f"  error: {error}")
        return 0, 0

    crate = data.get("crate", {})
    versions = data.get("versions", [])
    all_time = crate.get("downloads", 0)
    recent = crate.get("recent_downloads", 0)

    # Cross-check "recent" against the daily series (cheap: a few hundred
    # bytes); prefer it when it's available since it's the more direct sum.
    series, series_error = fetch_json(CRATE_DOWNLOADS_URL, CRATES_USER_AGENT)
    if not series_error and series:
        daily_total = sum(d.get("downloads", 0) for d in series.get("version_downloads", []))
        recent = daily_total

    if versions:
        headers = ["Version", "Downloads"]
        rows = [[v.get("num", "?"), str(v.get("downloads", 0))] for v in versions]
        print_table(headers, rows)
        print()
    print(f"  All-time downloads:      {all_time}")
    print(f"  Recent (last 90 days):   {recent}")
    return all_time, recent


GOATCOUNTER_BASE = "https://alix.goatcounter.com/api/v0"


def goatcounter_section() -> int | None:
    """Visitor counts, only when GOATCOUNTER_TOKEN is set. Returns the
    30-day pageview total, or None when skipped/failed."""
    print()
    print(bold("Visitors") + "  (alix.goatcounter.com)")
    token = os.environ.get("GOATCOUNTER_TOKEN", "").strip()
    if not token:
        print("  skipped: set GOATCOUNTER_TOKEN to include visitor counts")
        print("  (create a read-statistics token at alix.goatcounter.com, Settings, API)")
        return None

    auth = {"Authorization": f"Bearer {token}"}
    today = datetime.date.today()
    month_ago = today - datetime.timedelta(days=30)

    def total_between(start: datetime.date, end: datetime.date) -> tuple[int | None, str | None]:
        url = f"{GOATCOUNTER_BASE}/stats/total?start={start.isoformat()}&end={end.isoformat()}"
        data, error = fetch_json(url, CRATES_USER_AGENT, auth)
        if error:
            return None, error
        return data.get("total", 0), None

    last_30, error = total_between(month_ago, today)
    if error:
        print(f"  error: {error}")
        return None
    # "All time" = since the counter went live; a fixed early start is fine.
    all_time, error = total_between(datetime.date(2026, 7, 1), today)
    if error:
        print(f"  error: {error}")
        return last_30

    print(f"  Pageviews, last 30 days: {last_30}")
    print(f"  Pageviews, all time:     {all_time}")
    return last_30


def main() -> int:
    print(bold("alix — download stats"))
    print()
    github_total = github_section()
    crates_total, _recent = crates_section()
    visitors_30d = goatcounter_section()

    print()
    visitors_note = f" · {visitors_30d} pageviews in 30 days" if visitors_30d is not None else ""
    print(f"Summary: {github_total} GitHub asset downloads + {crates_total} crates.io downloads "
          f"= {github_total + crates_total} total{visitors_note}")
    print()
    print("Note: crates.io downloads are mostly automated (mirrors, CI caches, security")
    print("scanners), not necessarily humans. GitHub per-asset downloads are closer to")
    print("an actual person grabbing a binary.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
