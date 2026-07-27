#!/bin/sh
set -eu

cd "$(dirname "$0")/.."

baseline="scripts/deps-duplicates.txt"
actual=$(mktemp)
trap 'rm -f "$actual"' EXIT HUP INT TERM

cargo tree --locked --offline -d --edges normal,build --prefix none --format '{p}' |
    awk 'BEGIN { RS=""; FS="\n" } { print $1 }' |
    awk '{
        name = $1
        version = $2
        sub(/^v/, "", version)
        split(version, parts, ".")
        family = parts[1] == "0" ? "0." parts[2] : parts[1]
        print name "@" family
    }' |
    sort |
    uniq -c |
    awk '{ print $2 " x" $1 }' >"$actual"

if ! diff -u "$baseline" "$actual"; then
    cat >&2 <<'EOF'
deps-check: duplicate compatibility families changed

Inspect `cargo tree -d --edges normal,build` and align compatible requirements
before accepting another compiled version. If every new family is unavoidable,
review the reason with the maintainer and update scripts/deps-duplicates.txt.
EOF
    exit 1
fi

echo "deps-check: duplicate compatibility families match the reviewed baseline"
