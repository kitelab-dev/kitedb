#!/bin/sh
# Installs KiteDB for the current user (~/.local) or, with PREFIX, system-wide:
#   ./install.sh                      -> ~/.local
#   ./install.sh --desktop-shortcut   -> also puts an icon on your desktop
#   sudo PREFIX=/usr/local ./install.sh
# Works from a release tarball or from the source tree after `cargo build --release`.
set -eu

HERE="$(cd "$(dirname "$0")" && pwd)"
PREFIX="${PREFIX:-$HOME/.local}"
SHORTCUT=0
for arg in "$@"; do
    case "$arg" in
        --desktop-shortcut) SHORTCUT=1 ;;
        *) echo "usage: $0 [--desktop-shortcut]" >&2; exit 2 ;;
    esac
done

if [ -f "$HERE/kitedb" ]; then
    BIN="$HERE/kitedb"; DATA="$HERE/data"
else
    BIN="$HERE/../target/release/kitedb"; DATA="$HERE/../data"
fi
[ -f "$BIN" ] || { echo "kitedb binary not found; run 'cargo build --release' first" >&2; exit 1; }

install -Dm755 "$BIN" "$PREFIX/bin/kitedb"
# Point Exec at the installed binary so it starts even when $PREFIX/bin is
# not on the desktop session's PATH.
ENTRY="$PREFIX/share/applications/com.kitelab.KiteDB.desktop"
mkdir -p "$(dirname "$ENTRY")"
sed "s|^Exec=kitedb$|Exec=$PREFIX/bin/kitedb|" "$DATA/com.kitelab.KiteDB.desktop" > "$ENTRY"
chmod 644 "$ENTRY"
install -Dm644 "$DATA/com.kitelab.KiteDB.metainfo.xml" "$PREFIX/share/metainfo/com.kitelab.KiteDB.metainfo.xml"
for size in 64x64 256x256; do
    install -Dm644 "$DATA/icons/hicolor/$size/apps/com.kitelab.KiteDB.png" \
        "$PREFIX/share/icons/hicolor/$size/apps/com.kitelab.KiteDB.png"
done
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q "$PREFIX/share/applications" || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$PREFIX/share/icons/hicolor" || true

if [ "$SHORTCUT" = 1 ]; then
    DESKTOP_DIR="$(xdg-user-dir DESKTOP 2>/dev/null || echo "$HOME/Desktop")"
    mkdir -p "$DESKTOP_DIR"
    # Executable so KDE and GNOME treat the launcher as trusted.
    install -Dm755 "$ENTRY" "$DESKTOP_DIR/com.kitelab.KiteDB.desktop"
    command -v gio >/dev/null 2>&1 && gio set "$DESKTOP_DIR/com.kitelab.KiteDB.desktop" metadata::trusted true 2>/dev/null || true
    echo "Added a desktop shortcut in $DESKTOP_DIR"
fi

echo "Installed KiteDB to $PREFIX"
case ":$PATH:" in *":$PREFIX/bin:"*) ;; *) echo "Note: $PREFIX/bin is not on your PATH." ;; esac
