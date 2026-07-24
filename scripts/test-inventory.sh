#!/bin/sh
set -eu

all=$(mktemp)
ignored=$(mktemp)
trap 'rm -f "$all" "$ignored"' EXIT HUP INT TERM

cargo test --quiet --workspace --all-targets -- --list --format terse >"$all"
cargo test --quiet --workspace --all-targets -- --ignored --list --format terse >"$ignored"

count_tests() {
    awk '/: test$/ { count++ } END { print count + 0 }' "$1"
}

total_count=$(count_tests "$all")
ignored_count=$(count_tests "$ignored")
default_count=$((total_count - ignored_count))

if test "$default_count" -lt 0; then
    echo "test-inventory: ignored count exceeds total count" >&2
    exit 1
fi

printf 'Rust tests: %s default + %s ignored = %s total (--workspace --all-targets)\n' \
    "$default_count" "$ignored_count" "$total_count"
