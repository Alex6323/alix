#!/usr/bin/env python3
"""Reconcile cargo-mutants misses against the argued allowlist.

Reads mutants.out/missed.txt and docs/mutants-allowlist.txt, prints
every miss as ALLOWED (with no effect on the verdict) or UNREGISTERED,
and exits 0 only when every miss is allowed. Allowlist entries that
matched nothing are reported as possibly stale, informationally: on a
diff-scoped run most entries are simply out of scope.
"""

import re
import sys

LINE_RE = re.compile(r"^(?P<file>[^:]+):(?P<line>\d+):\d+: (?P<desc>.*)$")


def main(missed_path: str, allowlist_path: str) -> int:
    with open(allowlist_path, encoding="utf-8") as handle:
        entries = [
            line.strip()
            for line in handle
            if line.strip() and not line.lstrip().startswith("#")
        ]
    with open(missed_path, encoding="utf-8") as handle:
        missed = [line.strip() for line in handle if line.strip()]

    used = set()
    unregistered = []
    for miss in missed:
        match = LINE_RE.match(miss)
        if not match:
            unregistered.append(miss)
            continue
        broad = f"{match['file']}: {match['desc']}"
        narrow = f"{match['file']}:{match['line']}: {match['desc']}"
        key = next((e for e in (narrow, broad) if e in entries), None)
        if key is None:
            unregistered.append(miss)
        else:
            used.add(key)
            print(f"allowed:      {miss}")

    for entry in entries:
        if entry not in used:
            print(f"note: allowlist entry matched nothing this run: {entry}")

    if unregistered:
        for miss in unregistered:
            print(f"UNREGISTERED: {miss}")
        print(
            f"mutants-allowed: {len(unregistered)} unregistered miss(es); "
            "kill them or argue them into docs/mutants-allowlist.txt"
        )
        return 1
    print(f"mutants-allowed: all {len(missed)} miss(es) are argued residuals")
    return 0


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit("usage: mutants_allowed.py <missed.txt> <allowlist>")
    sys.exit(main(sys.argv[1], sys.argv[2]))
