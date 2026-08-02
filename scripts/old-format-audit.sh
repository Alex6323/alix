#!/bin/sh
set -eu

CDPATH=''
export CDPATH
repo_root=$(cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

version=$(sed -n 's/^version = "\([^"]*\)"$/\1/p' Cargo.toml | head -n 1)
case "$version" in
    0.*) ;;
    *)
        {
            echo "old-format-audit: package version is $version."
            echo "This temporary pre-1.0 gate must not survive 1.0. Remove it:"
            echo "  - the old-format-audit Makefile target"
            echo "  - scripts/old-format-audit.sh"
            echo "  - scripts/old-format-audit-prompt.md"
            echo "  - the RELEASING.md gate entry"
            echo "Blocking the release until the removal lands."
        } >&2
        exit 1
        ;;
esac

cli=${OLD_FORMAT_AUDIT_CLI:-claude}
model=${OLD_FORMAT_AUDIT_MODEL:-opus}
effort=${OLD_FORMAT_AUDIT_EFFORT:-high}
report=${OLD_FORMAT_AUDIT_REPORT:-target/old-format-audit.md}
prompt=scripts/old-format-audit-prompt.md

command -v "$cli" >/dev/null 2>&1 || {
    echo "old-format-audit: '$cli' is required and must be authenticated" >&2
    exit 1
}

mkdir -p "$(dirname -- "$report")"
tmp_report="${report}.tmp"
tmp_json="${report}.json.tmp"
trap 'rm -f "$tmp_report" "$tmp_json"' EXIT HUP INT TERM

echo "old-format-audit: live read-only LLM audit (model=$model, effort=$effort)"
echo "old-format-audit: report -> $report"

{
    cat "$prompt"
    printf '\n## Repository state\n\n'
    printf 'Commit: `%s`\n\n' "$(git rev-parse HEAD)"
    if test -n "$(git status --porcelain)"; then
        printf 'The worktree is dirty. Audit the current working tree, including these changes:\n\n```text\n'
        git status --short
        printf '```\n\n'
    else
        printf 'The worktree is clean.\n\n'
    fi
    printf '## Audit manifest\n\n```text\n'
    git ls-files -- src web mobile/alix/rust/src mobile/alix/lib tests Makefile scripts |
        grep -v '^mobile/alix/lib/src/rust/' || true
    printf '```\n'
} | "$cli" \
    --safe-mode \
    --print \
    --output-format json \
    --no-session-persistence \
    --permission-mode dontAsk \
    --allowedTools Read Glob Grep \
    --model "$model" \
    --effort "$effort" >"$tmp_json"

# The json envelope carries the report text plus per-run usage; surface the
# cost of every run instead of hiding it in a flag nobody passes.
python3 - "$tmp_json" "$tmp_report" <<'PYEOF'
import json, sys

doc = json.load(open(sys.argv[1]))
if isinstance(doc, list):
    doc = next(item for item in reversed(doc) if item.get("type") == "result")
with open(sys.argv[2], "w") as out:
    out.write(doc["result"])
u = doc.get("usage", {})
parts = [
    f"in={u.get('input_tokens', '?')}",
    f"out={u.get('output_tokens', '?')}",
    f"cache-read={u.get('cache_read_input_tokens', 0)}",
    f"cache-write={u.get('cache_creation_input_tokens', 0)}",
]
cost = doc.get("total_cost_usd")
if cost is not None:
    parts.append(f"cost=${cost:.2f}")
ms = doc.get("duration_ms")
if ms is not None:
    parts.append(f"took={ms / 60000:.1f}min")
print("old-format-audit: tokens " + " ".join(parts))
PYEOF

mv "$tmp_report" "$report"
rm -f "$tmp_json"
trap - EXIT HUP INT TERM
cat "$report"

verdict=$(sed -n '1p' "$report")
case "$verdict" in
    "OLD-FORMAT AUDIT: PASS")
        echo "old-format-audit: PASS"
        ;;
    "OLD-FORMAT AUDIT: FAIL")
        echo "old-format-audit: FAIL — remove the recognition machinery and rerun before release" >&2
        exit 1
        ;;
    *)
        echo "old-format-audit: invalid verdict; expected OLD-FORMAT AUDIT: PASS or FAIL" >&2
        exit 1
        ;;
esac
