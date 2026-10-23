#!/usr/bin/env bash
# A session of its own for the e2e tests and the bench: a nested Hyprland
# (tests/e2e/hypr.lua, one 1920×1080 output), its own runtime folder, its own
# D-Bus session bus (the tray and notifications stay out of the real panel),
# and kadr's folders under it. The nested window is kept off screen in the
# real session (a special workspace) and still renders.
#
#   eval "$(scripts/nested.sh start)"   # prints `export KEY=…` lines
#   scripts/nested.sh stop "$KADR_E2E_DIR"
#
# Needs a running Hyprland to nest in.
set -euo pipefail
cd "$(dirname "$0")/.."

start() {
  [ -n "${WAYLAND_DISPLAY:-}" ] && [ -n "${HYPRLAND_INSTANCE_SIGNATURE:-}" ] || { echo "nested.sh: not inside Hyprland" >&2; exit 2; }
  local parent_display="$WAYLAND_DISPLAY"
  [[ $parent_display == /* ]] || parent_display="$XDG_RUNTIME_DIR/$parent_display"
  # Short: Unix socket paths end at 108 bytes (Hyprland cuts its own a few
  # bytes sooner), and Hyprland's go two
  # folders deeper with a long instance name.
  local dir
  dir=$(mktemp -d "${XDG_RUNTIME_DIR:-/tmp}/ke-XXXX")
  chmod 700 "$dir"
  mkdir -p "$dir/home/config" "$dir/home/cache" "$dir/home/data" "$dir/home/shots" "$dir/home/videos"
  # The user's folders too, or kadr finds their real recordings in ~/Videos
  # (the history lists them).
  printf 'XDG_VIDEOS_DIR="%s"\nXDG_PICTURES_DIR="%s"\n' "$dir/home/videos" "$dir/home/shots" >"$dir/home/config/user-dirs.dirs"

  # In the real session: the nested window floats at the output's size on a
  # special workspace, and keeps getting frames there.
  hyprctl eval 'hl.window_rule({ match = { class = "^aquamarine$" }, float = true, size = "1920 1080", workspace = "special:kadr-e2e silent", render_unfocused = true })' >/dev/null 2>&1 \
    || hyprctl keyword windowrule 'float,size 1920 1080,workspace special:kadr-e2e silent,renderunfocused,class:^(aquamarine)$' >/dev/null 2>&1 || true

  # A bus that starts no services: with the session's own config it would
  # start the desktop portal on demand, in the real session's environment
  # (attached to the real Hyprland), which then crashed when this bus went
  # away, once per test session.
  cat >"$dir/bus.conf" <<'CONF'
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <!-- Required; the address option below says where. -->
  <listen>unix:tmpdir=/tmp</listen>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
CONF
  local bus
  bus=$(dbus-daemon --config-file="$dir/bus.conf" --fork --print-address=1 --print-pid=3 --address="unix:path=$dir/bus" 3>"$dir/dbus.pid")

  env -u HYPRLAND_INSTANCE_SIGNATURE XDG_RUNTIME_DIR="$dir" WAYLAND_DISPLAY="$parent_display" DBUS_SESSION_BUS_ADDRESS="$bus" \
    setsid start-hyprland -- -c "$PWD/tests/e2e/hypr.lua" >"$dir/hyprland.out" 2>&1 &
  echo $! >"$dir/hyprland.pid"

  # Up when its socket answers and the output has its size.
  local sig="" ok=""
  for _ in $(seq 1 100); do
    sig=$(ls "$dir/hypr" 2>/dev/null | head -1 || true)
    if [ -n "$sig" ] && [ -S "$dir/hypr/$sig/.socket.sock" ] \
      && XDG_RUNTIME_DIR="$dir" HYPRLAND_INSTANCE_SIGNATURE="$sig" hyprctl monitors 2>/dev/null | grep -q "1920x1080"; then
      ok=1
      break
    fi
    sleep 0.1
  done
  if [ -z "$ok" ]; then
    echo "nested.sh: the nested Hyprland didn't come up:" >&2
    tail -5 "$dir/hyprland.out" >&2 || true
    stop "$dir"
    exit 1
  fi
  # Nothing of Hyprland's own on screen: its notices would land in captures.
  XDG_RUNTIME_DIR="$dir" HYPRLAND_INSTANCE_SIGNATURE="$sig" hyprctl dismissnotify >/dev/null 2>&1 || true
  # Frames come: every capture waits for one. The real session stops
  # drawing the hidden window while one of its own special workspaces (a
  # scratchpad) is shown, and then every grim would hang.
  local frames=""
  for _ in $(seq 1 20); do
    if [ "$(XDG_RUNTIME_DIR="$dir" WAYLAND_DISPLAY=wayland-1 timeout 1 grim -g "0,0 1x1" -t ppm - 2>/dev/null | wc -c)" -gt 0 ]; then
      frames=1
      break
    fi
    sleep 0.2
  done
  if [ -z "$frames" ]; then
    echo "nested.sh: the nested Hyprland gets no frames; is a special workspace (a scratchpad) shown in the real session? Close it and try again." >&2
    stop "$dir"
    exit 1
  fi
  cat <<VARS
export KADR_E2E_DIR=$dir
export XDG_RUNTIME_DIR=$dir
export WAYLAND_DISPLAY=wayland-1
export HYPRLAND_INSTANCE_SIGNATURE=$sig
export DBUS_SESSION_BUS_ADDRESS=$bus
export XDG_CONFIG_HOME=$dir/home/config
export XDG_CACHE_HOME=$dir/home/cache
export XDG_DATA_HOME=$dir/home/data
export OMARCHY_SCREENSHOT_DIR=$dir/home/shots
export KADR_DEV=1
VARS
}

stop() {
  local dir="${1:?stop needs the nested session folder}"
  [[ $(basename "$dir") == ke-* ]] || { echo "nested.sh: $dir isn't a nested session" >&2; exit 2; }
  # kadr and anything else started in the session, by its runtime folder;
  # not this script or the shell that ran it, which may carry it too.
  local mine=" $$ " q=$$
  while [ "$q" -gt 1 ]; do
    q=$(awk '{print $4}' "/proc/$q/stat" 2>/dev/null || echo 1)
    mine+="$q "
  done
  for p in /proc/[0-9]*; do
    [[ $mine == *" ${p#/proc/} "* ]] && continue
    if { tr '\0' '\n' <"$p/environ"; } 2>/dev/null | grep -qx "XDG_RUNTIME_DIR=$dir"; then
      kill "${p#/proc/}" 2>/dev/null || true
    fi
  done
  for f in hyprland.pid dbus.pid; do
    [ -f "$dir/$f" ] && kill "$(cat "$dir/$f")" 2>/dev/null || true
  done
  sleep 0.3
  rm -rf "$dir"
}

case "${1:-}" in
  start) start ;;
  stop) stop "${2:-}" ;;
  *) echo "usage: scripts/nested.sh start | stop <dir>" >&2; exit 2 ;;
esac
