#!/usr/bin/env bash
# Build dist/Triceratops-<version>-<arch>.AppImage.
#
#   scripts/package-linux.sh                  build release binary + AppImage
#   SKIP_BUILD=1 scripts/package-linux.sh     reuse target/release/triceratops
#   APPIMAGETOOL=/path/to/appimagetool scripts/package-linux.sh
#
# The binary only links libc, libm, libstdc++ and libgcc_s; graphics drivers
# (Vulkan/GL, X11/Wayland) are loaded from the host at runtime, so nothing
# else is bundled. Build on the oldest distro you want to support (CI uses
# Ubuntu 22.04, glibc 2.35).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
ARCH="$(uname -m)"
VERSION="${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}"

if [[ -z "${SKIP_BUILD:-}" ]]; then
    cargo build --release --locked --bin triceratops
fi

DIST="$ROOT/dist"
APPDIR="$ROOT/target/appimage/Triceratops.AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/icons/hicolor/256x256/apps" "$DIST"

install -m755 target/release/triceratops "$APPDIR/usr/bin/triceratops"
strip "$APPDIR/usr/bin/triceratops" || true
install -m644 packaging/linux/triceratops.desktop "$APPDIR/usr/share/applications/"
install -m644 packaging/linux/triceratops.png "$APPDIR/usr/share/icons/hicolor/256x256/apps/"
cp packaging/linux/triceratops.desktop packaging/linux/triceratops.png "$APPDIR/"
ln -sf triceratops.png "$APPDIR/.DirIcon"
cat > "$APPDIR/AppRun" <<'APPRUN'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/bin/triceratops" "$@"
APPRUN
chmod +x "$APPDIR/AppRun"

TOOL="${APPIMAGETOOL:-$(command -v appimagetool || true)}"
if [[ -z "$TOOL" ]]; then
    TOOL="$ROOT/target/appimage/appimagetool-$ARCH.AppImage"
    if [[ ! -x "$TOOL" ]]; then
        echo "downloading appimagetool..." >&2
        curl -fsSL -o "$TOOL" "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$ARCH.AppImage"
        chmod +x "$TOOL"
    fi
fi

OUT="$DIST/Triceratops-$VERSION-$ARCH.AppImage"
# APPIMAGE_EXTRACT_AND_RUN: works where FUSE is unavailable (containers, CI)
APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$ARCH" "$TOOL" --no-appstream "$APPDIR" "$OUT"
echo "built $OUT"
