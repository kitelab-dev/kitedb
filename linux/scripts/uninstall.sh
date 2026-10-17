#!/bin/sh
# Removes what install.sh installed (same PREFIX). Saved connections in
# ~/.config/kitedb and keyring passwords are kept.
set -eu
PREFIX="${PREFIX:-$HOME/.local}"
rm -f "$PREFIX/bin/kitedb" \
    "$PREFIX/share/applications/com.kitelab.KiteDB.desktop" \
    "$PREFIX/share/metainfo/com.kitelab.KiteDB.metainfo.xml" \
    "$PREFIX/share/icons/hicolor/64x64/apps/com.kitelab.KiteDB.png" \
    "$PREFIX/share/icons/hicolor/256x256/apps/com.kitelab.KiteDB.png"
DESKTOP_DIR="$(xdg-user-dir DESKTOP 2>/dev/null || echo "$HOME/Desktop")"
rm -f "$DESKTOP_DIR/com.kitelab.KiteDB.desktop"
echo "Removed KiteDB from $PREFIX"
