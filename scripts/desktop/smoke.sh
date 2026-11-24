#!/bin/bash
# Desktop smoke test: run the real app (Tauri + WebKitGTK) on a virtual display, step through
# the stages and the main screens, and screenshot each step. Needs Xvfb, xdotool and
# ImageMagick (`import`). Coordinates assume the default 1680 × 1050 layout.
#
#   cargo build --release -p vcsd-app            # embeds ui/dist (run `npm --prefix ui run build` first)
#   ./target/release/vcsd demo /tmp/demo
#   scripts/desktop/smoke.sh /tmp/shots /tmp/demo
set -u
OUT=$1; PROJ=$2; APP=${APP:-$(dirname "$0")/../../target/release/vc-sound-design}
mkdir -p "$OUT"
export DISPLAY=:99
Xvfb :99 -screen 0 1680x1050x24 -nolisten tcp >/dev/null 2>&1 &
XV=$!
for i in $(seq 1 50); do [ -e /tmp/.X11-unix/X99 ] && break; sleep 0.2; done
WEBKIT_DISABLE_COMPOSITING_MODE=1 "$APP" "$PROJ" > "$OUT/app.log" 2>&1 &
APPPID=$!
WID=""
for i in $(seq 1 120); do WID=$(xdotool search --name "^VC Sound Design$" 2>/dev/null | head -1); [ -n "$WID" ] && break; sleep 0.5; done
[ -z "$WID" ] && { echo "no window (see $OUT/app.log)"; kill $APPPID $XV; exit 1; }
xdotool windowmove "$WID" 0 0 windowsize "$WID" 1680 1050
sleep 6
shot() { import -window root "$OUT/$1.png"; echo "shot $1"; }
key() { xdotool windowactivate --sync "$WID" 2>/dev/null; xdotool key --window "$WID" "$@"; sleep 1.2; }
click() { xdotool mousemove "$1" "$2" click 1; sleep 1.2; }
for s in 1 2 3 4 5 6; do key $s; shot "stage-$s"; done
key 3
click 246 50; shot menu                                     # project menu (must open above the timeline)
click 291 423; shot preferences                             # ☰ › Preferences…
click 1233 816
click 246 50; click 275 481; sleep 2; shot plugins           # ☰ › Plugins…
xdotool key Escape
kill $APPPID; sleep 1; kill $XV
echo "done: $OUT"
