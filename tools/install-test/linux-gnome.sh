#!/bin/sh
# Real run on GNOME's window manager (SQ-48). GNOME on Wayland has no layer-shell, so the tray
# is a frameless window kept below the others through XWayland. This test starts Mutter, the
# window manager of GNOME, as a Wayland session on a virtual monitor, installs the package and
# reads from the window manager what the panel's window really is: on the desktop by default,
# an ordinary window when the Floating window setting is on, and back. Nothing is simulated.
#
#   sh linux-gnome.sh PACKAGE.tar.gz
set -eu
PACKAGE=$1
BIN=$HOME/.local/bin/status-quo
LOG=/tmp/panel.log
fail() {
  printf '\nFAILED: %s\n' "$*"
  echo '--- panel'
  cat "$LOG" 2>/dev/null || true
  echo '--- mutter'
  tail -n 30 /tmp/mutter.log 2>/dev/null || true
  exit 1
}
step() { printf '\n######## %s\n' "$*"; }
# wait_for SECONDS COMMAND...: true as soon as the command succeeds.
wait_for() {
  seconds=$1
  shift
  while [ "$seconds" -gt 0 ]; do
    if "$@" >/dev/null 2>&1; then return 0; fi
    seconds=$((seconds - 1))
    sleep 1
  done
  return 1
}

step "GNOME's window manager (Mutter) as a Wayland session on a virtual monitor"
sudo apt-get update >/dev/null
sudo apt-get install -y mutter xwayland x11-utils dbus-x11 >/dev/null
XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/tmp/runtime-$(id -u)}
mkdir -p "$XDG_RUNTIME_DIR" && chmod 700 "$XDG_RUNTIME_DIR"
export XDG_RUNTIME_DIR XDG_SESSION_TYPE=wayland XDG_CURRENT_DESKTOP=GNOME
unset DISPLAY WAYLAND_DISPLAY || true
eval "$(dbus-launch --sh-syntax)"
mutter --headless --wayland --virtual-monitor 1280x800 --wayland-display sq-gnome >/tmp/mutter.log 2>&1 &
wait_for 30 test -S "$XDG_RUNTIME_DIR/sq-gnome" || fail "Mutter did not start"
export WAYLAND_DISPLAY=sq-gnome
mutter --version
# Mutter offers an X display for X programs (XWayland) and protects it with a key file, as a
# GNOME session does; the session passes both to every program through DISPLAY and XAUTHORITY.
display() { sed -n 's/.*Using public X11 display \(:[0-9]*\).*/\1/p' /tmp/mutter.log | head -n 1; }
authority() {
  for file in "$XDG_RUNTIME_DIR"/.mutter-Xwaylandauth.*; do
    [ -f "$file" ] && echo "$file" && return 0
  done
  return 1
}
has_display() { [ -n "$(display)" ] && authority; }
wait_for 30 has_display || fail "Mutter offers no XWayland display"
DISPLAY=$(display)
XAUTHORITY=$(authority)
export DISPLAY XAUTHORITY
echo "Wayland display: $WAYLAND_DISPLAY, XWayland display: $DISPLAY"

step "Install the package (no start yet)"
mkdir -p /tmp/package && tar -xzf "$PACKAGE" -C /tmp/package
sh /tmp/package/status-quo-linux-*/install.sh --no-launch || fail "the installer failed"
"$BIN" stop >/dev/null 2>&1 || true

# The panel's window as the window manager sees it.
window() { xprop -root _NET_CLIENT_LIST 2>/dev/null | sed -n 's/.*# //p' | tr ',' '\n' | tr -d ' ' | head -n 1; }
state() { xprop -id "$(window)" _NET_WM_STATE 2>/dev/null; }
framed() { xprop -id "$(window)" _MOTIF_WM_HINTS 2>/dev/null | grep -q '= 0x2, 0x0, 0x1,'; }
on_desktop() { state | grep -q _NET_WM_STATE_BELOW && state | grep -q _NET_WM_STATE_SKIP_TASKBAR && ! framed; }
ordinary() { framed && ! state | grep -q _NET_WM_STATE_SKIP_TASKBAR; }
panel_pid() { ps -eo pid,args | awk -v bin="$BIN" '$2 == bin && NF == 2 { print $1 }' | head -n 1; }

step "Default: the panel sits on the desktop background"
"$BIN" >"$LOG" 2>&1 &
wait_for 40 grep -q 'X11 desktop hints active' "$LOG" || fail "Mutter did not accept the desktop placement"
grep -q 'Status Quo host: x11-desktop: Wayland without layer-shell' "$LOG" || fail "the panel did not choose the desktop placement by itself"
wait_for 10 on_desktop || fail "the window is not on the desktop: $(state)"
state
PID=$(panel_pid)
[ -n "$PID" ] || fail "the panel is not running"
"$BIN" doctor || fail "the compatibility check reports a blocking problem"

step "Floating window on: an ordinary window with a title bar"
"$BIN" floating on || fail "the setting could not be changed"
wait_for 40 ordinary || fail "the panel did not become an ordinary window: $(state)"
grep -q 'Floating window setting is on' "$LOG" || fail "the panel did not say why it is a window"
state
[ "$(panel_pid)" = "$PID" ] || fail "the panel did not restart in place (process $PID became $(panel_pid))"

step "Floating window off: back on the desktop background"
"$BIN" floating off || fail "the setting could not be changed"
wait_for 40 on_desktop || fail "the panel did not go back to the desktop: $(state)"
state
[ "$(panel_pid)" = "$PID" ] || fail "the panel did not restart in place"

step "Uninstall"
"$BIN" uninstall || fail "uninstall failed"
cat "$LOG"
printf '\nPROVEN on %s with Mutter %s (GNOME on Wayland, through XWayland)\n' "$(sed -n 's/^PRETTY_NAME=//p' /etc/os-release)" "$(mutter --version | awk '{print $2}')"
