#!/bin/sh
# Structural guard for CHANGELOG.md: a truncation or mangled section layout
# fails loud in the inner loop. Semantic accuracy stays with docs-audit; this
# only proves the skeleton is intact.
set -eu

file="CHANGELOG.md"
fail() { echo "changelog-check: $1" >&2; exit 1; }

[ -f "$file" ] || fail "$file is missing"

headings=$(grep -c '^## \[' "$file" || true)
unreleased=$(grep -c '^## \[Unreleased\]' "$file" || true)

[ "$unreleased" -eq 1 ] || fail "expected exactly one '## [Unreleased]' heading, found $unreleased"
first=$(grep '^## \[' "$file" | head -n 1)
[ "$first" = "## [Unreleased]" ] || fail "the first release heading must be [Unreleased], found '$first'"
[ "$headings" -ge 2 ] || fail "only $headings release heading(s); the released history is missing"

dupes=$(grep '^## \[' "$file" | LC_ALL=C sort | LC_ALL=C uniq -d)
[ -z "$dupes" ] || fail "duplicate release headings: $dupes"

# One Added/Changed/Fixed per release: merge residue duplicates a subsection
# instead of appending to it, and the split content reads as two half-lists.
section_dupes=$(awk '
    /^## \[/ { release = $0; delete seen }
    /^### /  {
        if ($0 in seen) { printf "%s repeats in %s\n", $0, release }
        seen[$0] = 1
    }
' "$file")
[ -z "$section_dupes" ] || fail "duplicate subsections:
$section_dupes"

# A dirty working tree must never lose release headings relative to HEAD:
# releases only ever append headings, so a decrease is a truncation.
if committed=$(git show "HEAD:$file" 2>/dev/null); then
    head_count=$(printf '%s\n' "$committed" | grep -c '^## \[' || true)
    [ "$headings" -ge "$head_count" ] || \
        fail "release headings decreased vs HEAD ($head_count -> $headings); the file looks truncated"
fi

echo "changelog-check: $headings release headings, skeleton intact"
