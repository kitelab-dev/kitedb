#!/usr/bin/env bash
# Packs build/Kitelab DB.app into build/KitelabDB.dmg with an Applications
# shortcut for drag-and-drop install.
set -euo pipefail

cd "$(dirname "$0")/.."
APP="build/Kitelab DB.app"
DMG="build/KitelabDB.dmg"
[[ -d "$APP" ]] || { echo "Run scripts/build-app.sh first" >&2; exit 1; }

STAGING="$(mktemp -d)"
trap 'rm -rf "$STAGING"' EXIT
cp -R "$APP" "$STAGING/"
ln -s /Applications "$STAGING/Applications"

rm -f "$DMG"
hdiutil create -volname "Kitelab DB" -srcfolder "$STAGING" -fs HFS+ -format UDZO -imagekey zlib-level=9 "$DMG"
shasum -a 256 "$DMG" | tee "$DMG.sha256"
