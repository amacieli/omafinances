#!/bin/bash
# Run a scratch (test) OmaFinances instance and keep its window on ONE fixed monitor.
#
#   scripts/omaf-scratch.sh [--port N] [--] [command...]
#
# With no command it runs the dev binary from the shared build dir. The window is pinned to:
#   1. the monitor the Vivaldi browser window (class vivaldi-stable) is on at launch time;
#   2. else the monitor remembered in ~/.config/omafinances/test-monitor (default HDMI-A-1);
# and that choice is remembered. Any scratch window titled "OmaFinances" is moved there as soon as it
# appears, without taking focus. Only windows of scratch instances are moved; the live one is never touched.
# Prints "pinned <address> -> <monitor>" lines; the instance's own exit status is returned.

set -u
CFG="${XDG_CONFIG_HOME:-$HOME/.config}/omafinances"
PIN_FILE="$CFG/test-monitor"
LIVE_DIR="${OMAFINANCES_LIVE_DIR:-/opt/omafinances}"
BIN="${OMAFINANCES_BUILD_DIR:-/mnt/d/omafinances-build}/debug/omafinances"
PORT=27185

if [ "${1:-}" = "--port" ]; then PORT="$2"; shift 2; fi
[ "${1:-}" = "--" ] && shift

export WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-$(ls "/run/user/$(id -u)" | grep -E '^wayland-[0-9]+$' | head -1)}"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
export QT_QPA_PLATFORM="${QT_QPA_PLATFORM:-wayland}"
if [ -z "${HYPRLAND_INSTANCE_SIGNATURE:-}" ]; then
  HYPRLAND_INSTANCE_SIGNATURE=$(ls -t "$XDG_RUNTIME_DIR/hypr" 2>/dev/null | head -1)
  export HYPRLAND_INSTANCE_SIGNATURE
fi

# The monitor the Vivaldi browser window (class vivaldi-stable; not its PWAs) is on right now.
vivaldi_monitor() {
  hyprctl clients -j | jq -r '.[] | select(.class=="vivaldi-stable") | .monitor' | head -1 | while read -r m; do
    hyprctl monitors -j | jq -r --argjson m "$m" '.[] | select(.id==$m) | .name'
  done
}

mkdir -p "$CFG"
TARGET=$(vivaldi_monitor | head -1)
[ -z "$TARGET" ] && TARGET=$(cat "$PIN_FILE" 2>/dev/null)
[ -z "$TARGET" ] && TARGET=HDMI-A-1
echo "$TARGET" > "$PIN_FILE"
if ! hyprctl monitors -j | jq -e --arg n "$TARGET" '.[] | select(.name==$n)' > /dev/null; then
  echo "omaf-scratch: monitor $TARGET is not connected" >&2; exit 2
fi

pin_loop() {
  while kill -0 "$1" 2>/dev/null; do
    # scratch windows only: the pid's command line must not be the live install
    hyprctl clients -j | jq -r '.[] | select(.title=="OmaFinances") | "\(.address) \(.pid) \(.monitor) \(.size[0]) \(.size[1])"' |
    while read -r addr pid mon _w _h; do
      tr '\0' ' ' < "/proc/$pid/cmdline" 2>/dev/null | grep -q -- "-p $LIVE_DIR/qml" && continue
      cur=$(hyprctl monitors -j | jq -r --argjson m "$mon" '.[] | select(.id==$m) | .name')
      [ "$cur" = "$TARGET" ] && continue
      ws=$(hyprctl monitors -j | jq -r --arg n "$TARGET" '.[] | select(.name==$n) | .activeWorkspace.id')
      # Hyprland 0.56 has a Lua dispatcher API; follow=false keeps your view and focus where they are.
      hyprctl eval "hl.dispatch(hl.dsp.window.move({ workspace = '$ws', window = 'address:$addr', follow = false }))" > /dev/null \
        && echo "pinned $addr -> $TARGET (workspace $ws)"
    done
    sleep 0.3
  done
}

if [ $# -eq 0 ]; then set -- "$BIN"; fi
OMAFINANCES_PORT="$PORT" "$@" &
APP=$!
pin_loop "$APP" &
PIN=$!
trap 'kill "$APP" 2>/dev/null' INT TERM
wait "$APP"; rc=$?
kill "$PIN" 2>/dev/null
exit $rc
