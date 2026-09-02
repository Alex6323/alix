#!/bin/sh
# Gate for `make publish`: the crate on crates.io must be the tree the GitHub
# Release was built from. The manifest version names the tag; that tag must
# exist, sit in HEAD's history, and be the same commit origin carries (the
# release workflow built from origin's tag), and the tree must be clean so
# nothing untracked can ride into the tarball.
set -eu

fail() { echo "publish-check: $1" >&2; exit 1; }

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
[ -n "$version" ] || fail "no version line in Cargo.toml"
tag="v$version"

dirty=$(git status --porcelain)
[ -z "$dirty" ] || fail "the tree is dirty; publish only from a clean tree:
$dirty"

local_sha=$(git rev-parse -q --verify "refs/tags/$tag^{commit}" 2>/dev/null || true)
[ -n "$local_sha" ] || fail "no tag $tag in this clone; Cargo.toml says $version but that release was never tagged"

git merge-base --is-ancestor "$local_sha" HEAD \
    || fail "tag $tag ($local_sha) is not in HEAD's history; publish from the branch that carries the release"

listing=$(git ls-remote --tags origin "refs/tags/$tag") \
    || fail "could not list origin's tags (network or SSH auth); nothing was published"
remote_sha=$(printf '%s\n' "$listing" | tail -n 1 | cut -f 1)
[ -n "$remote_sha" ] || fail "origin has no tag $tag; the release workflow builds from origin's tag, push it first"
[ "$remote_sha" = "$local_sha" ] \
    || fail "tag $tag is $local_sha here but $remote_sha on origin; the two releases would differ"

echo "publish-check: OK ($tag = $local_sha, on origin, in HEAD's history, tree clean)"
