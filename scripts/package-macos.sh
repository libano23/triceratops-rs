#!/usr/bin/env bash
# Build dist/Triceratops-<version>-macos-universal.dmg (Apple Silicon + Intel).
# Must run on macOS with the Xcode command line tools and CMake installed.
#
#   scripts/package-macos.sh
#   TARGETS=aarch64-apple-darwin scripts/package-macos.sh   single arch
#
# The app is ad-hoc signed only: on first launch users right-click > Open
# (or run `xattr -dr com.apple.quarantine /Applications/Triceratops.app`).
# Set MACOS_SIGN_IDENTITY to a "Developer ID Application: ..." identity to
# sign properly; notarization is a separate step.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"
TARGETS="${TARGETS:-aarch64-apple-darwin x86_64-apple-darwin}"
VERSION="${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}"

bins=()
for t in $TARGETS; do
    rustup target add "$t" >/dev/null
    cargo build --release --locked --bin triceratops --target "$t"
    bins+=("target/$t/release/triceratops")
done
suffix="universal"
[[ ${#bins[@]} -eq 1 ]] && suffix="${TARGETS%%-*}"

DIST="$ROOT/dist"
STAGE="$ROOT/target/macos"
APP="$STAGE/Triceratops.app"
rm -rf "$STAGE"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$DIST"

lipo -create -output "$APP/Contents/MacOS/triceratops" "${bins[@]}"
sed "s/@VERSION@/$VERSION/g" packaging/macos/Info.plist > "$APP/Contents/Info.plist"

ICONSET="$ROOT/target/triceratops.iconset"
rm -rf "$ICONSET"
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
    sips -s format png -z $s $s packaging/macos/icon-1024.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
    d=$((s * 2))
    sips -s format png -z $d $d packaging/macos/icon-1024.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/triceratops.icns"

codesign --force --deep --options runtime --sign "${MACOS_SIGN_IDENTITY:--}" "$APP"

DMG="$DIST/Triceratops-$VERSION-macos-$suffix.dmg"
rm -f "$DMG"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname Triceratops -srcfolder "$STAGE" -ov -format UDZO "$DMG"
echo "built $DMG"
