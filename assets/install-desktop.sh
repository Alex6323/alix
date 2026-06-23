#!/usr/bin/env bash
# Installs the alix desktop icon, launcher, and .desktop entry into the
# per-user XDG locations, so the app appears in the desktop menu (Cinnamon,
# GNOME, KDE, ...). Idempotent: safe to re-run after changing the SVG.
#
# Requires: rsvg-convert (librsvg). Optional: gtk-update-icon-cache,
# update-desktop-database (caches are refreshed if present).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project="$(dirname "$here")"
svg="$here/alix.svg"

data="${XDG_DATA_HOME:-$HOME/.local/share}"
icons="$data/icons/hicolor"
apps="$data/applications"
bindir="$HOME/.local/bin"

echo "Installing icons from $svg"
install -Dm644 "$svg" "$icons/scalable/apps/alix.svg"
for s in 16 22 24 32 48 64 128 256; do
    mkdir -p "$icons/${s}x${s}/apps"
    rsvg-convert -w "$s" -h "$s" "$svg" -o "$icons/${s}x${s}/apps/alix.png"
done

echo "Installing launcher to $bindir/alix-launch"
mkdir -p "$bindir"
cat > "$bindir/alix-launch" <<EOF
#!/usr/bin/env bash
# Desktop launcher for alix: opens the deck picker (no arguments), so you
# choose which decks to review.
set -u

PROJECT="$project"
BIN="\$(type -P alix 2>/dev/null || true)"
if [ -z "\$BIN" ]; then
    if [ -x "\$PROJECT/target/release/alix" ]; then
        BIN="\$PROJECT/target/release/alix"
    elif [ -x "\$PROJECT/target/debug/alix" ]; then
        BIN="\$PROJECT/target/debug/alix"
    fi
fi
if [ -z "\$BIN" ]; then
    echo "alix: could not find 'alix' (try: cargo install --path '\$PROJECT')." >&2
    read -n1 -rsp \$'Press any key to close…\n'
    exit 1
fi

"\$BIN"
echo
read -n1 -rsp \$'Press any key to close…\n'
EOF
chmod 755 "$bindir/alix-launch"

echo "Installing desktop entry to $apps/alix.desktop"
mkdir -p "$apps"
cat > "$apps/alix.desktop" <<EOF
[Desktop Entry]
Type=Application
Version=1.1
Name=alix
GenericName=Flashcard Trainer
Comment=Spaced-repetition flashcard trainer for the terminal
Exec=$bindir/alix-launch
Icon=alix
Terminal=true
Categories=Education;Languages;
Keywords=flashcard;srs;learning;spaced repetition;anki;review;
StartupNotify=false
EOF

command -v gtk-update-icon-cache >/dev/null 2>&1 \
    && gtk-update-icon-cache -f -t "$icons" >/dev/null 2>&1 || true
command -v update-desktop-database >/dev/null 2>&1 \
    && update-desktop-database "$apps" >/dev/null 2>&1 || true

echo "Done. 'alix' should appear in your application menu."
