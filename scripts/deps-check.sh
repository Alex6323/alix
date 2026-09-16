#!/bin/sh
set -eu

cd "$(dirname "$0")/.."

cargo_deny_version="0.20.2"
if ! command -v cargo-deny >/dev/null 2>&1; then
    echo "deps-check: cargo-deny $cargo_deny_version is required; found no cargo-deny" >&2
    exit 1
fi
actual_version=$(cargo-deny --version)
if [ "$actual_version" != "cargo-deny $cargo_deny_version" ]; then
    echo "deps-check: cargo-deny $cargo_deny_version is required; found $actual_version" >&2
    exit 1
fi

cargo-deny --manifest-path Cargo.toml --locked --offline --config deny.toml \
    check -D no-license-field -D unmatched-skip -D unnecessary-skip licenses bans
python3 scripts/check-dependency-policy.py
