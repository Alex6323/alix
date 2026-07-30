#!/bin/sh
# Rewrite every pinned Rust toolchain literal from the version files. The
# workflows cannot read rust-toolchain.toml themselves: dtolnay/rust-toolchain
# declares `toolchain` as a required input and errors when it is empty.
set -eu

WORKFLOWS='.github/workflows/ci.yml .github/workflows/release.yml .github/workflows/mobile-release.yml'
STABLE_PIN='^\( *\)toolchain: [0-9][0-9.]*$'
NIGHTLY_PIN='^\( *\)toolchain: nightly-[0-9][0-9-]*$'

RUST=${RUST:-}
NIGHTLY=${NIGHTLY:-}

if [ -z "$RUST" ] && [ -z "$NIGHTLY" ]; then
    echo 'usage: make bump-rust [RUST=X.Y.Z] [NIGHTLY=nightly-YYYY-MM-DD]' >&2
    exit 1
fi

if [ -n "$RUST" ] && ! echo "$RUST" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
    echo "RUST must be X.Y.Z (it is pinned into release workflows), got: $RUST" >&2
    exit 1
fi

if [ -n "$NIGHTLY" ] && ! echo "$NIGHTLY" | grep -Eq '^nightly-[0-9]{4}-[0-9]{2}-[0-9]{2}$'; then
    echo "NIGHTLY must be nightly-YYYY-MM-DD, got: $NIGHTLY" >&2
    exit 1
fi

rewrite() {
    sed "$2" "$1" > "$1.tmp"
    mv "$1.tmp" "$1"
}

if [ -n "$RUST" ]; then
    old=$(sed -n 's/^channel = "\([^"]*\)"$/\1/p' rust-toolchain.toml)
    rewrite rust-toolchain.toml "s/^channel = \"[^\"]*\"$/channel = \"$RUST\"/"
    printf '  %-26s %s -> %s\n' rust-toolchain.toml "$old" "$RUST"
fi

if [ -n "$NIGHTLY" ]; then
    old=$(cat .rust-nightly-version)
    echo "$NIGHTLY" > .rust-nightly-version
    printf '  %-26s %s -> %s\n' .rust-nightly-version "$old" "$NIGHTLY"
fi

for workflow in $WORKFLOWS; do
    changed=''
    if [ -n "$RUST" ]; then
        hits=$(grep -c "$STABLE_PIN" "$workflow" || true)
        if [ "$hits" -gt 0 ]; then
            rewrite "$workflow" "s/$STABLE_PIN/\\1toolchain: $RUST/"
            changed="$hits stable"
        fi
    fi
    if [ -n "$NIGHTLY" ]; then
        hits=$(grep -c "$NIGHTLY_PIN" "$workflow" || true)
        if [ "$hits" -gt 0 ]; then
            rewrite "$workflow" "s/$NIGHTLY_PIN/\\1toolchain: $NIGHTLY/"
            [ -n "$changed" ] && changed="$changed, "
            changed="$changed$hits nightly"
        fi
    fi
    [ -n "$changed" ] && printf '  %-26s %s\n' "$(basename "$workflow")" "$changed"
done

echo
echo '  running toolchain-check ...'
sh scripts/toolchain-check.sh > /dev/null && echo '  ok' || {
    echo '  FAILED: run `make toolchain-check` to see which pin disagrees' >&2
    exit 1
}
