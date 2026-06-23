#!/bin/sh
# alix installer — downloads a pre-compiled binary from GitHub Releases.
#
#   curl -sSf https://alix.study/install.sh | sh
#
# No Rust toolchain required. macOS and Linux (x86-64) are covered here; on
# Windows, grab the .zip from the releases page, or use `cargo install alix`.
set -eu

REPO="Alex6323/alix"
BIN="alix"

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
      *) echo "alix: unsupported Linux arch: $arch — try: cargo install alix" >&2; exit 1 ;;
    esac ;;
  *)
    echo "alix: unsupported OS: $os — on Windows, download the .zip from" >&2
    echo "      https://github.com/$REPO/releases/latest" >&2
    exit 1 ;;
esac

asset="${BIN}-${target}.tar.gz"
url="https://github.com/${REPO}/releases/latest/download/${asset}"
bindir="${ALIX_BIN_DIR:-$HOME/.local/bin}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "alix: downloading $asset"
curl -sSfL "$url" -o "$tmp/$asset"
tar -xzf "$tmp/$asset" -C "$tmp"

# The binary sits at the archive root (alongside the bundled licenses/README).
binpath="$(find "$tmp" -type f -name "$BIN" | head -n1)"
[ -n "$binpath" ] || { echo "alix: could not find '$BIN' in the archive" >&2; exit 1; }

mkdir -p "$bindir"
install -m 755 "$binpath" "$bindir/$BIN"
echo "alix: installed to $bindir/$BIN"

case ":$PATH:" in
  *":$bindir:"*) ;;
  *) echo "alix: add it to your PATH —  export PATH=\"$bindir:\$PATH\"" ;;
esac
echo "alix: run  $BIN --help  to get started"
