#!/bin/sh
set -eu

CDPATH=''
export CDPATH
repo_root=$(cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

cli=${DOCS_AUDIT_CLI:-claude}
model=${DOCS_AUDIT_MODEL:-opus}
effort=${DOCS_AUDIT_EFFORT:-high}
report=${DOCS_AUDIT_REPORT:-target/docs-audit.md}
prompt=scripts/docs-audit-prompt.md

is_visual() {
    case "$1" in
        *.avif|*.gif|*.jpeg|*.jpg|*.png|*.svg|*.webp) return 0 ;;
        *) return 1 ;;
    esac
}

text_manifest() {
    {
        # A pathspec without a slash matches every directory. Keep this broad:
        # a new tracked Markdown surface must enter the release audit without
        # someone remembering to extend an allowlist.
        git ls-files -- '*.md'
        git ls-files -- site | while IFS= read -r path; do
            if ! is_visual "$path"; then
                printf '%s\n' "$path"
            fi
        done
    } | sort -u
}

visual_manifest() {
     git ls-files -- site docs/book docs/examples assets/alix.svg | while IFS= read -r path; do
        if is_visual "$path"; then
            printf '%s\n' "$path"
        fi
    done
}

if [ "${DOCS_AUDIT_MANIFEST_ONLY:-}" = text ]; then
    text_manifest
    exit 0
fi

command -v "$cli" >/dev/null 2>&1 || {
    echo "docs-audit: '$cli' is required and must be authenticated" >&2
    exit 1
}

mkdir -p "$(dirname -- "$report")"
tmp_report="${report}.tmp"
trap 'rm -f "$tmp_report"' EXIT HUP INT TERM

echo "docs-audit: live read-only LLM audit (model=$model, effort=$effort)"
echo "docs-audit: report -> $report"

{
    cat "$prompt"
    printf '\n## Repository state\n\n'
    printf "Commit: \`%s\`\n\n" "$(git rev-parse HEAD)"
    if test -n "$(git status --porcelain)"; then
        printf 'The worktree is dirty. Audit the current working tree, including these changes:\n\n```text\n'
        git status --short
        printf '```\n\n'
    else
        printf 'The worktree is clean.\n\n'
    fi

    printf '## Tracked text manifest\n\n```text\n'
    text_manifest
    printf '```\n\n'

    printf '## Tracked visual manifest\n\n```text\n'
    visual_manifest
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
    "DOCS AUDIT: PASS")
        echo "docs-audit: PASS"
        ;;
    "DOCS AUDIT: FAIL")
        echo "docs-audit: FAIL — resolve the report and rerun before release" >&2
        exit 1
        ;;
    *)
        echo "docs-audit: invalid verdict; expected DOCS AUDIT: PASS or FAIL" >&2
        exit 1
        ;;
esac
