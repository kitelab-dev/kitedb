#!/usr/bin/env bash
# Builds "build/Kitelab DB.app": a release binary plus the Homebrew client
# libraries it links (libpq, libmysqlclient and their dependencies) copied
# into Contents/Frameworks, so the app runs without Homebrew.
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(pwd)"
APP_NAME="Kitelab DB"
EXECUTABLE="KitelabDB"
APP="$ROOT/build/$APP_NAME.app"
VERSION="${VERSION:-1.0}"

# SwiftUI's macros ship with Xcode, not the Command Line Tools.
if [[ -z "${DEVELOPER_DIR:-}" && "$(xcode-select -p)" == *CommandLineTools* && -d /Applications/Xcode.app ]]; then
    export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
fi

swift build -c release --product "$EXECUTABLE"
BIN_DIR="$(swift build -c release --show-bin-path)"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$APP/Contents/Frameworks"
cp "$BIN_DIR/$EXECUTABLE" "$APP/Contents/MacOS/$EXECUTABLE"
[[ -f Resources/AppIcon.icns ]] || swift scripts/make-icon.swift Resources
cp Resources/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$APP_NAME</string>
    <key>CFBundleDisplayName</key><string>$APP_NAME</string>
    <key>CFBundleIdentifier</key><string>com.kitelab.dbclient</string>
    <key>CFBundleExecutable</key><string>$EXECUTABLE</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>LSMinimumSystemVersion</key><string>14.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSPrincipalClass</key><string>NSApplication</string>
</dict>
</plist>
PLIST

FRAMEWORKS="$APP/Contents/Frameworks"

# Copy every non-system dylib a binary links, recursively, and point the
# load commands at @rpath (= Contents/Frameworks).
bundle_deps() {
    local binary="$1"
    local dep name
    for dep in $(otool -L "$binary" | tail -n +2 | awk '{print $1}' | grep -E '^(/opt/homebrew|/usr/local)/' || true); do
        name="$(basename "$dep")"
        if [[ ! -f "$FRAMEWORKS/$name" ]]; then
            cp "$dep" "$FRAMEWORKS/$name"
            chmod u+w "$FRAMEWORKS/$name"
            install_name_tool -id "@rpath/$name" "$FRAMEWORKS/$name" 2>/dev/null
            bundle_deps "$FRAMEWORKS/$name"
        fi
        install_name_tool -change "$dep" "@rpath/$name" "$binary" 2>/dev/null
    done
}

bundle_deps "$APP/Contents/MacOS/$EXECUTABLE"
for lib in "$FRAMEWORKS"/*.dylib; do
    install_name_tool -add_rpath "@loader_path" "$lib" 2>/dev/null || true
done

# The app can only run where every bundled library can, so advertise the
# highest deployment target among them.
MIN_OS="$(for f in "$APP/Contents/MacOS/$EXECUTABLE" "$FRAMEWORKS"/*.dylib; do
    otool -l "$f" | awk '/LC_BUILD_VERSION/ { found = 1 } found && /minos/ { print $2; found = 0 }'
done | sort -V | tail -1)"
plutil -replace LSMinimumSystemVersion -string "$MIN_OS" "$APP/Contents/Info.plist"

# License texts of the bundled libraries.
LICENSES="$APP/Contents/Resources/Licenses"
mkdir -p "$LICENSES"
cp ../LICENSE "$LICENSES/KitelabDB-LICENSE.txt"
cp THIRD_PARTY_NOTICES.md "$LICENSES/"
for formula in libpq mysql-client mysql openssl@3 krb5 zstd zlib-ng-compat zlib; do
    prefix="$(brew --prefix "$formula" 2>/dev/null)" || continue
    for file in "$prefix"/{LICENSE,LICENSE.txt,LICENSE.md,COPYING,NOTICE}; do
        [[ -f "$file" ]] && cp "$file" "$LICENSES/$formula-$(basename "$file")"
    done
done

codesign --force --sign - "$FRAMEWORKS"/*.dylib
codesign --force --sign - "$APP"

echo "Built $APP (macOS $MIN_OS+)"
echo "Bundled libraries:"
ls "$FRAMEWORKS"
