#!/usr/bin/env bash
# Headless UI screenshots of the real app (Xvfb + software Vulkan), for reviewing layout.
#   scripts/ui-shots.sh <out-dir> [scan-path]
# Env: SPACE_BIN (default target/release/space-scanner), SPACE_FAKE_HOME (isolated $HOME,
#      put old caches under it to populate Cleanup), CLEANUP_WAIT (seconds, default 20).
# Needs: Xvfb, xdotool, python3 + Pillow, mesa-vulkan-drivers (lavapipe), libxkbcommon-x11.
set -euo pipefail
OUT=${1:?out dir}; SCAN=${2:-/usr/share}
BIN=${SPACE_BIN:-./target/release/space-scanner}
export DISPLAY=${DISPLAY:-:99}
export VK_ICD_FILENAMES=${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.json}
export XDG_RUNTIME_DIR=$(mktemp -d); chmod 700 "$XDG_RUNTIME_DIR"
export HOME=${SPACE_FAKE_HOME:-$(mktemp -d)}
mkdir -p "$OUT"
pgrep -x Xvfb >/dev/null || { Xvfb "$DISPLAY" -screen 0 1600x1000x24 >/dev/null 2>&1 & sleep 1; }

W=1280; H=800
shot() { python3 -c "
from PIL import ImageGrab
ImageGrab.grab(xdisplay='$DISPLAY').crop((0,0,$W,$H)).save('$OUT/$1.png')"; }
click() { xdotool mousemove "$1" "$2" click 1; sleep "${3:-1.2}"; }
launch() {            # launch [path] — start the app and size its window
  pkill -x space-scanner 2>/dev/null || true; sleep 0.5
  "$BIN" "$@" >>"$OUT/app.log" 2>&1 &
  sleep "${LAUNCH_WAIT:-7}"
  WIN=$(xdotool search --name "Space Scanner" | head -1)
  xdotool windowsize "$WIN" "$W" "$H"; xdotool windowmove "$WIN" 0 0; sleep 2
}
resize() { W=$1; H=$2; xdotool windowsize "$WIN" "$W" "$H"; sleep 2; }

launch;                shot 01-scan
launch "$SCAN";        sleep 4; shot 02-usage
click 150 420;         shot 03-usage-selected
click 1126 337;        shot 04-usage-queued           # Add to review (details panel)
resize 1000 700;       shot 05-usage-medium
resize 760 560;        shot 06-usage-narrow-list
click 91 84;           shot 07-usage-narrow-map       # List | Map toggle
click 602 27;          shot 08-review                 # Review pill
click 658 521;         shot 09-confirm                # Move to Trash… (confirmation, not confirmed)
xdotool key Escape; sleep 1
pkill -x space-scanner || true

W=1280; H=800
launch;                click 300 27 "${CLEANUP_WAIT:-20}"; shot 10-cleanup
click 58 255;          shot 11-cleanup-selected       # first category checkbox
click 1126 292;        shot 12-review-wide            # Review N items
pkill -x space-scanner || true
