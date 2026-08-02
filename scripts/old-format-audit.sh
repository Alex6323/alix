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
trap 'rm -f "$tmp_report"' EXIT HUP INT TERM

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
    --output-format text \
    --no-session-persistence \
    --permission-mode dontAsk \
    --allowedTools Read Glob Grep \
    --model "$model" \
    --effort "$effort" >"$tmp_report"

mv "$tmp_report" "$report"
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
