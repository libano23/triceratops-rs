#!/usr/bin/env bash
# Capture a Triceratops screenshot for the website.
#
#   site/tools/shoot.sh <name> <script.txt> [locust-file] [seconds]
#
# Runs the release build inside a private rootful Xwayland server, so the
# window always has its default size (1680 x 1020) whatever your window
# manager does, with a throw-away config directory (default layout, your
# own settings untouched). The command script runs, and after a few
# seconds (so windows have faded in and Locust has solved) the X screen is
# grabbed into target/shots/<name>.png. Needs Xwayland and ImageMagick.
# SHOOT_OPTIONS=<options.json> starts it with those settings instead of the defaults.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
name="$1"; script="$2"; locust="${3:-}"; seconds="${4:-4}"
cd "$ROOT"
mkdir -p target/shots
out="target/shots/$name.png"
rm -f "$out"

cfg="$(mktemp -d)"
mkdir -p "$cfg/triceratops"
[[ -n "$locust" ]] && cp "$locust" "$cfg/triceratops/locust.locust"
cp "$script" "$cfg/script.txt"
[[ -n "${SHOOT_OPTIONS:-}" ]] && cp "$SHOOT_OPTIONS" "$cfg/triceratops/options.json"

display=":$(( 40 + RANDOM % 50 ))"
Xwayland "$display" -geometry 1920x1200 -noreset >/dev/null 2>&1 &
xpid=$!
app=""
cleanup() {
    if [[ -n "$app" ]]; then kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true; fi
    kill "$xpid" 2>/dev/null || true
    wait "$xpid" 2>/dev/null || true
    rm -rf "$cfg"
}
trap cleanup EXIT
sleep 1.5

env -u WAYLAND_DISPLAY DISPLAY="$display" XDG_CONFIG_HOME="$cfg" RUST_LOG=error \
    target/release/triceratops --script "$cfg/script.txt" >/dev/null 2>&1 &
app=$!
sleep "$seconds"
# the window is placed at the top-left corner of the empty X screen
DISPLAY="$display" magick import -silent -window root -crop 1680x1020+0+0 +repage "$out"
[[ -s "$out" ]] && echo "$out" || { echo "no capture for $name" >&2; exit 1; }
