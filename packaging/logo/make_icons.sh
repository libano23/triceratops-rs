#!/usr/bin/env bash
# Build every icon from the rendered head (packaging/logo/head.png):
#   packaging/logo/icon.png            1024 px app icon (head on a dark rounded square)
#   packaging/linux/triceratops.png    256 px
#   packaging/macos/icon-1024.png      1024 px
#   packaging/windows/triceratops.ico  16-256 px
#   site/assets/{logo.png,favicon-32.png,apple-touch-icon.png}
# Re-render the head first with:
#   blender -b --python packaging/logo/render_head.py -- packaging/logo/head.png 1024
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."
L=packaging/logo
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# dark rounded square with a soft radial light, then the head on top
magick -size 1024x1024 radial-gradient:'#22344d'-'#0b1120' "$tmp/bg.png"
magick -size 1024x1024 xc:none -fill white -draw "roundrectangle 0,0 1023,1023 224,224" "$tmp/mask.png"
magick "$tmp/bg.png" "$tmp/mask.png" -alpha off -compose CopyOpacity -composite "$tmp/plate.png"
magick "$L/head.png" -resize 880x880 "$tmp/head.png"
magick "$tmp/plate.png" "$tmp/head.png" -gravity center -geometry +0+10 -compose over -composite "$L/icon.png"

magick "$L/icon.png" -resize 256x256 packaging/linux/triceratops.png
cp "$L/icon.png" packaging/macos/icon-1024.png
for s in 16 24 32 48 64 128 256; do magick "$L/icon.png" -resize ${s}x${s} "$tmp/i$s.png"; done
magick "$tmp"/i{16,24,32,48,64,128,256}.png packaging/windows/triceratops.ico

mkdir -p site/assets
magick "$L/head.png" -resize 512x512 site/assets/logo.png
magick "$L/icon.png" -resize 32x32 site/assets/favicon-32.png
magick "$L/icon.png" -resize 180x180 site/assets/apple-touch-icon.png
echo "icons written"
