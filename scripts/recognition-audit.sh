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
            echo "recognition-audit: package version is $version."
            echo "This temporary pre-1.0 gate must not survive 1.0. Remove it:"
            echo "  - the recognition-audit Makefile target"
            echo "  - scripts/recognition-audit.sh"
            echo "  - scripts/recognition-audit-prompt.md"
            echo "  - the RELEASING.md gate entry"
            echo "Blocking the release until the removal lands."
        } >&2
        exit 1
        ;;
esac

cli=${RECOGNITION_AUDIT_CLI:-claude}
model=${RECOGNITION_AUDIT_MODEL:-opus}
effort=${RECOGNITION_AUDIT_EFFORT:-high}
report=${RECOGNITION_AUDIT_REPORT:-target/recognition-audit.md}
prompt=scripts/recognition-audit-prompt.md

command -v "$cli" >/dev/null 2>&1 || {
    echo "recognition-audit: '$cli' is required and must be authenticated" >&2
    exit 1
}

mkdir -p "$(dirname -- "$report")"
tmp_report="${report}.tmp"
trap 'rm -f "$tmp_report"' EXIT HUP INT TERM

echo "recognition-audit: live read-only LLM audit (model=$model, effort=$effort)"
echo "recognition-audit: report -> $report"

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
    "RECOGNITION AUDIT: PASS")
        echo "recognition-audit: PASS"
        ;;
    "RECOGNITION AUDIT: FAIL")
        echo "recognition-audit: FAIL — remove the recognition machinery and rerun before release" >&2
        exit 1
        ;;
    *)
        echo "recognition-audit: invalid verdict; expected RECOGNITION AUDIT: PASS or FAIL" >&2
        exit 1
        ;;
esac
