#!/bin/sh
# alix installer - downloads a verified pre-compiled binary from GitHub Releases.
#
#   curl -sSf https://alix.study/install.sh | sh
#
# No Rust toolchain required. macOS and Linux (x86-64) are covered here; on
# Windows, grab the .zip from the releases page, or use `cargo install alix`.
set -efu

REPO="Alex6323/alix"
BIN="alix"

die() {
  echo "alix: $*" >&2
  exit 1
}

os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Darwin)
    case "$arch" in
      arm64 | aarch64) target="aarch64-apple-darwin" ;;
      x86_64) target="x86_64-apple-darwin" ;;
      *) echo "alix: unsupported macOS arch: $arch" >&2; exit 1 ;;
    esac ;;
  Linux)
    case "$arch" in
      x86_64) target="x86_64-unknown-linux-gnu" ;;
      *) die "unsupported Linux arch: $arch - try: cargo install alix" ;;
    esac ;;
  *)
    echo "alix: unsupported OS: $os - on Windows, download the .zip from" >&2
    echo "      https://github.com/$REPO/releases/latest" >&2
    exit 1 ;;
esac

asset="${BIN}-${target}.tar.gz"
checksum_asset="${BIN}-${target}.sha256"
release_base="${ALIX_RELEASE_BASE_URL:-https://github.com/${REPO}}"
release_base="${release_base%/}"
version="${ALIX_VERSION:-}"

if [ -z "$version" ]; then
  latest_url="${release_base}/releases/latest"
  if ! resolved="$(curl -sSfL -o /dev/null -w '%{url_effective}' "$latest_url")"; then
    die "could not resolve the latest release"
  fi
  resolved="${resolved%/}"
  tag_prefix="${release_base}/releases/tag/"
  case "$resolved" in
    "$tag_prefix"*) version="${resolved#"$tag_prefix"}" ;;
    *) die "latest release resolved outside the expected tag path: $resolved" ;;
  esac
fi

if ! printf '%s\n' "$version" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z][0-9A-Za-z.-]*)?$'; then
  die "invalid release tag: $version"
fi

release_url="${release_base}/releases/download/${version}"
url="${release_url}/${asset}"
checksum_url="${release_url}/${checksum_asset}"
bindir="${ALIX_BIN_DIR:-$HOME/.local/bin}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "alix: downloading $asset from $version"
if ! curl -sSfL "$url" -o "$tmp/$asset"; then
  die "could not download $asset from $version"
fi
if ! curl -sSfL "$checksum_url" -o "$tmp/$checksum_asset"; then
  die "could not download checksum $checksum_asset from $version"
fi

line_count="$(awk 'END { print NR }' "$tmp/$checksum_asset")"
[ "$line_count" = 1 ] || die "checksum file must contain exactly one record"
checksum_line="$(sed -n '1p' "$tmp/$checksum_asset")"
set -- $checksum_line
[ "$#" = 2 ] || die "checksum file has an invalid record"
published_digest="$1"
published_name="${2#\*}"
[ "$published_name" = "$asset" ] || \
  die "checksum file does not name the requested archive: $published_name"
case "$published_digest" in
  '' | *[!0-9A-Fa-f]*) die "checksum file does not contain a SHA-256 digest" ;;
esac
[ "${#published_digest}" = 64 ] || \
  die "checksum file does not contain a 64-character SHA-256 digest"
published_digest="$(printf '%s' "$published_digest" | tr 'A-F' 'a-f')"

actual_digest=""
if command -v sha256sum >/dev/null 2>&1; then
  actual_digest="$(sha256sum "$tmp/$asset")"
  actual_digest="${actual_digest%% *}"
elif command -v shasum >/dev/null 2>&1; then
  actual_digest="$(shasum -a 256 "$tmp/$asset")"
  actual_digest="${actual_digest%% *}"
elif [ "${ALIX_INSTALL_UNVERIFIED:-0}" = 1 ]; then
  echo "alix: WARNING: installing UNVERIFIED because ALIX_INSTALL_UNVERIFIED=1" >&2
else
  die "no SHA-256 tool found; install sha256sum/shasum or explicitly set ALIX_INSTALL_UNVERIFIED=1"
fi

if [ -n "$actual_digest" ] && [ "$actual_digest" != "$published_digest" ]; then
  die "checksum mismatch for $asset"
fi

tar -xzf "$tmp/$asset" -C "$tmp"

# The binary sits at the archive root (alongside the bundled licenses/README).
binpath="$(find "$tmp" -type f -name "$BIN" | head -n1)"
[ -n "$binpath" ] || { echo "alix: could not find '$BIN' in the archive" >&2; exit 1; }

mkdir -p "$bindir"
install -m 755 "$binpath" "$bindir/$BIN"
echo "alix: installed to $bindir/$BIN"

case ":${PATH:-}:" in
  *":$bindir:"*) ;;
  *) echo "alix: add it to your PATH -  export PATH=\"$bindir:\$PATH\"" ;;
esac
echo "alix: run  $BIN --help  to get started"
