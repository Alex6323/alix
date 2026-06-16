#!/usr/bin/env bash
# Installs the flash desktop icon, launcher, and .desktop entry into the
# per-user XDG locations, so the app appears in the desktop menu (Cinnamon,
# GNOME, KDE, ...). Idempotent: safe to re-run after changing the SVG.
#
# Requires: rsvg-convert (librsvg). Optional: gtk-update-icon-cache,
# update-desktop-database (caches are refreshed if present).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project="$(dirname "$here")"
svg="$here/flash.svg"

data="${XDG_DATA_HOME:-$HOME/.local/share}"
icons="$data/icons/hicolor"
apps="$data/applications"
bindir="$HOME/.local/bin"

echo "Installing icons from $svg"
install -Dm644 "$svg" "$icons/scalable/apps/flash.svg"
for s in 16 22 24 32 48 64 128 256; do
    mkdir -p "$icons/${s}x${s}/apps"
    rsvg-convert -w "$s" -h "$s" "$svg" -o "$icons/${s}x${s}/apps/flash.png"
done

echo "Installing launcher to $bindir/flash-launch"
mkdir -p "$bindir"
cat > "$bindir/flash-launch" <<EOF
#!/usr/bin/env bash
# Desktop launcher for flash: opens the deck picker (no arguments), so you
# choose which decks to review.
set -u

PROJECT="$project"
BIN="\$(type -P flash 2>/dev/null || true)"
if [ -z "\$BIN" ]; then
    if [ -x "\$PROJECT/target/release/flash" ]; then
        BIN="\$PROJECT/target/release/flash"
    elif [ -x "\$PROJECT/target/debug/flash" ]; then
        BIN="\$PROJECT/target/debug/flash"
    fi
fi
if [ -z "\$BIN" ]; then
    echo "flash: could not find 'flash' (try: cargo install --path '\$PROJECT')." >&2
    read -n1 -rsp \$'Press any key to close…\n'
    exit 1
fi

"\$BIN"
echo
read -n1 -rsp \$'Press any key to close…\n'
EOF
chmod 755 "$bindir/flash-launch"

echo "Installing desktop entry to $apps/flash.desktop"
mkdir -p "$apps"
cat > "$apps/flash.desktop" <<EOF
[Desktop Entry]
Type=Application
Version=1.1
Name=flash
GenericName=Flashcard Trainer
Comment=Spaced-repetition flashcard trainer for the terminal
Exec=$bindir/flash-launch
Icon=flash
Terminal=true
Categories=Education;Languages;
Keywords=flashcard;srs;learning;spaced repetition;anki;review;
StartupNotify=false
EOF

command -v gtk-update-icon-cache >/dev/null 2>&1 \
    && gtk-update-icon-cache -f -t "$icons" >/dev/null 2>&1 || true
command -v update-desktop-database >/dev/null 2>&1 \
    && update-desktop-database "$apps" >/dev/null 2>&1 || true

echo "Done. 'flash' should appear in your application menu."
