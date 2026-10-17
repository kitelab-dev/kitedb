#!/bin/sh
# Builds build/KiteDB-linux-<arch>.tar.gz: the release binary, desktop entry,
# icons and install.sh. Needs GTK 4 and libadwaita on the target system.
set -eu

cd "$(dirname "$0")/.."
ARCH="$(uname -m)"
NAME="KiteDB-linux-$ARCH"
STAGE="build/$NAME"

# VERSION (e.g. from the release tag) is shown in the app; defaults to Cargo.toml's.
if [ -n "${VERSION:-}" ]; then export KITEDB_VERSION="$VERSION"; fi
cargo build --release --locked
rm -rf "$STAGE" "build/$NAME.tar.gz"
mkdir -p "$STAGE"
cp target/release/kitedb "$STAGE/"
cp -r data "$STAGE/data"
cp scripts/install.sh scripts/uninstall.sh "$STAGE/"
cp ../LICENSE THIRD_PARTY_NOTICES.md "$STAGE/"
tar -C build -czf "build/$NAME.tar.gz" "$NAME"
(cd build && sha256sum "$NAME.tar.gz" > "$NAME.tar.gz.sha256")
echo "build/$NAME.tar.gz"
