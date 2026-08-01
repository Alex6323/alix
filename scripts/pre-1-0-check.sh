#!/bin/sh
set -eu

version=$(sed -n 's/^version = "\([^"]*\)"$/\1/p' Cargo.toml | head -n 1)
case "$version" in
    0.*) ;;
    *)
        echo "pre-1.0-check: package version is $version; gate not applicable"
        exit 0
        ;;
esac

pattern='legacy|compat|deprecated|sentinel|adopt|coexist|migrat(e|es|ed|ing|ion|ions|or|ors)|older[[:space:]_-]+clients|graceful[[:space:]_-]+upgrade|(^|[^[:alnum:]])shim([^[:alnum:]]|$)'
roots='src assets/web mobile/alix/rust/src mobile/alix/lib'
report=$(mktemp)
trap 'rm -f "$report"' EXIT HUP INT TERM

set +e
rg -n -i --color never \
    --glob '!mobile/alix/lib/src/rust/**' \
    "$pattern" \
    $roots >"$report"
content_status=$?

rg --files \
    --glob '!mobile/alix/lib/src/rust/**' \
    $roots |
    rg -n -i --color never "$pattern" >>"$report"
path_status=$?
set -e

if [ "$content_status" -gt 1 ] || [ "$path_status" -gt 1 ]; then
    echo "pre-1.0-check: rg failed" >&2
    exit 2
fi

if [ "$content_status" -eq 0 ] || [ "$path_status" -eq 0 ]; then
    printf '%s\n' \
        "pre-1.0-check: forbidden backwards-compatibility vocabulary in production code:" >&2
    cat "$report" >&2
    echo "Remove the compatibility design. Do not rename it to evade this gate." >&2
    exit 1
fi

echo "pre-1.0-check: production code contains only the current design"
