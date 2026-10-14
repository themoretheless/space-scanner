#!/usr/bin/env bash
# Headless UI screenshots of the real app (Xvfb + software Vulkan), for reviewing layout.
#   scripts/ui-shots.sh <out-dir> [scan-path]
# Without a scan path it builds a small sample home folder (projects, downloads,
# caches) inside the isolated $HOME and scans that, so Disk usage, Cleanup and
# the Trash list all have something to show. System folders such as /usr are
# protected and cannot enter the Trash list.
# Env: SPACE_BIN (default target/release/space-scanner), SPACE_FAKE_HOME (isolated $HOME),
#      CLEANUP_WAIT (seconds for the junk check, default 90).
# Needs: Xvfb, xdotool, python3 + Pillow, mesa-vulkan-drivers (lavapipe), libxkbcommon-x11.
set -euo pipefail
OUT=${1:?out dir}
BIN=${SPACE_BIN:-./target/release/space-scanner}
export DISPLAY=${DISPLAY:-:99}
export VK_ICD_FILENAMES=${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.json}
export XDG_RUNTIME_DIR=$(mktemp -d); chmod 700 "$XDG_RUNTIME_DIR"
export HOME=${SPACE_FAKE_HOME:-$(mktemp -d)}
mkdir -p "$OUT"
if ! pgrep -x Xvfb >/dev/null; then
  Xvfb "$DISPLAY" -screen 0 1600x1000x24 >/dev/null 2>&1 &
  for _ in $(seq 40); do xdotool getdisplaygeometry >/dev/null 2>&1 && break; sleep 0.25; done
fi

sample_home() {        # projects, downloads and caches, all 40 days old
  local h=$HOME mk
  mk() { mkdir -p "$(dirname "$1")"; dd if=/dev/urandom of="$1" bs=1M count="$2" status=none; }
  mk "$h/Developer/app/target/debug/app" 24; mk "$h/Developer/app/target/release/app" 12
  mk "$h/Developer/app/src/main.rs" 1; mk "$h/Documents/old-backup.zip" 10
  mk "$h/Downloads/installer.iso" 16; mk "$h/Pictures/trip.mov" 8
  for i in 1 2 3; do mk "$h/.cache/pip/http/blob$i" 2; mk "$h/.npm/_cacache/c$i" 1; done
  find "$h" -exec touch -h -d '40 days ago' {} +
}
if [ -n "${2:-}" ]; then SCAN=$2; else sample_home; SCAN=$HOME; fi

W=1280; H=800
shot() { python3 -c "
from PIL import ImageGrab
ImageGrab.grab(xdisplay='$DISPLAY').crop((0,0,$W,$H)).save('$OUT/$1.png')"; }
click() { xdotool mousemove "$1" "$2" click 1; sleep "${3:-1.2}"; }
# Refocus only when needed: a needless focus change drops modifier state (⌘/Ctrl chords).
focus() { [ "$(xdotool getwindowfocus 2>/dev/null)" = "$WIN" ] || xdotool windowfocus --sync "$WIN" 2>/dev/null || true; }
key() { focus; xdotool key "$1"; sleep "${2:-1}"; }
launch() {            # launch [path] — start the app, size and focus its window
  pkill -x space-scanner 2>/dev/null || true; sleep 0.5
  "$BIN" "$@" >>"$OUT/app.log" 2>&1 &
  sleep "${LAUNCH_WAIT:-7}"
  WIN=$(xdotool search --name "Space Scanner" | head -1)
  xdotool windowsize "$WIN" "$W" "$H"; xdotool windowmove "$WIN" 0 0
  xdotool windowfocus --sync "$WIN" 2>/dev/null || true; sleep 2
}
resize() { W=$1; H=$2; xdotool windowsize "$WIN" "$W" "$H"; sleep 2; focus; }

# Sidebar at 1280 × 800: Overview 95, Disk usage 145, Cleanup 197, Trash list 247.
# 1. Overview, then Disk usage with a selection and a Trash list addition
launch;                shot 01-overview
launch "$SCAN";        sleep 4; shot 02-usage
click 330 239;         shot 03-usage-selected
key space 1.5;         shot 04-usage-queued               # Space adds to the Trash list (toast)
resize 1000 700;       shot 05-usage-medium               # details become a drawer
resize 760 560;        shot 06-usage-narrow-list          # icon rail, one pane, bottom sheet
key Escape;            click 718 29; shot 07-usage-narrow-map   # close details, List | Map

# 2. Trash list and the confirmation (not confirmed)
resize 1280 800
click 112 247;         shot 08-trash
click 1149 760;        shot 09-confirm
key Escape

# 3. Cleanup: intro, a check, Select all safe
click 112 197;         shot 10-cleanup-intro
click 752 395 "${CLEANUP_WAIT:-90}"; shot 11-cleanup
click 843 131;         shot 12-cleanup-safe

# 4. Settings, light appearance, shortcuts
key ctrl+comma 1.5;    shot 13-settings
click 490 228;         key Escape
click 112 145;         shot 14-usage-light
key shift+slash 1.5;   shot 15-shortcuts
pkill -x space-scanner || true
