#!/bin/sh
# Gate for `make publish`: the crate on crates.io must be the tree the GitHub
# Release was built from, so HEAD has to carry the tag the manifest version
# names and the tree has to be clean. `cargo publish` itself would package
# whatever tree it runs in under the manifest version.
set -eu

fail() { echo "publish-check: $1" >&2; exit 1; }

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
[ -n "$version" ] || fail "no version line in Cargo.toml"
tag="v$version"

dirty=$(git status --porcelain)
[ -z "$dirty" ] || fail "the tree is dirty; publish only from a clean checkout of $tag:
$dirty"

tags=$(git tag --points-at HEAD)
echo "$tags" | grep -qx "$tag" \
    || fail "HEAD carries no tag $tag (tags on HEAD: ${tags:-none}); git checkout $tag first"

echo "publish-check: OK (HEAD is $tag with a clean tree)"
