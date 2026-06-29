#!/usr/bin/env sh
# Release heartbeat — nudge when shipped work has piled up unreleased.
#
# Prints the number of entries under `## [Unreleased]` in CHANGELOG.md and the
# days since the last `vX.Y.Z` tag, then flags when a release looks due: there is
# unreleased work AND it has been at least DUE_DAYS since the last release.
# Informational only (always exits 0). Run from CLAUDE.md at the start of a
# session; the policy it backstops lives in RELEASING.md.
set -u

DUE_DAYS=28

# Count the top-level `- ` entries under the first `## [Unreleased]` heading,
# stopping at the next `## [` (a version section). Matches the project's rule
# that every user-facing change gets an [Unreleased] entry.
entries=$(awk '
  /^## \[Unreleased\]/ { inblk = 1; next }
  inblk && /^## \[/    { exit }
  inblk && /^- /       { n++ }
  END { print n + 0 }
' CHANGELOG.md)

# Days since the most recent vX.Y.Z tag (GNU `date -d`; "?" if unavailable).
last_tag=$(git describe --tags --abbrev=0 --match 'v*' 2>/dev/null || true)
days="?"
if [ -n "${last_tag:-}" ]; then
  tag_date=$(git log -1 --format=%cd --date=short "$last_tag" 2>/dev/null || true)
  if [ -n "${tag_date:-}" ]; then
    secs=$(date -d "$tag_date" +%s 2>/dev/null || true)
    [ -n "${secs:-}" ] && days=$(( ( $(date +%s) - secs ) / 86400 ))
  fi
else
  last_tag="(no tags)"
fi

if [ "$days" = "?" ]; then
  printf 'Release heartbeat: %s unreleased entries · last release %s\n' "$entries" "$last_tag"
else
  printf 'Release heartbeat: %s unreleased entries · last release %s (%s days ago)\n' "$entries" "$last_tag" "$days"
fi

if [ "$entries" -gt 0 ] && [ "$days" != "?" ] && [ "$days" -ge "$DUE_DAYS" ]; then
  printf '→ a release looks due — see RELEASING.md\n'
elif [ "$entries" -gt 0 ]; then
  printf '→ unreleased work, but under %s days since the last release — not yet due\n' "$DUE_DAYS"
else
  printf '→ nothing unreleased — not due\n'
fi
