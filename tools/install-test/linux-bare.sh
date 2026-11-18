#!/bin/sh
# Real install test on a bare Linux system (SQ-47). Runs as root inside a container made from a
# distribution's base image, where none of the program's libraries exist. Nothing is simulated:
# the one-paste command from the README is run as written, the system's real package manager
# installs the real libraries, and the real panel is started on a virtual screen.
#
#   sh linux-bare.sh COMMAND [OUT_DIR]
#
# COMMAND is the install line of the README (tools/install-test/readme-command.sh prints it).
# With SQ_RELEASE_BASE set, the package is taken from a local server (a build under test);
# without it, from the published release.
# A test that passes prints "PROVEN" as its last line. Any failure ends with exit code 1.
set -eu
COMMAND=$1
OUT=${2:-/tmp}
FETCH=${FETCH:-any}
LOG=/tmp/install.log
BIN=$HOME/.local/bin/status-quo
unset DISPLAY WAYLAND_DISPLAY || true

step() { printf '\n######## %s\n' "$*"; }
fail() {
  printf '\nFAILED: %s\n' "$*"
  [ -f "$HOME/.local/state/status-quo/panel.log" ] && tail -n 30 "$HOME/.local/state/status-quo/panel.log"
  exit 1
}
have() { command -v "$1" >/dev/null 2>&1; }
# Install packages with whatever this system has; each name is tried alone so that a name one
# distribution lacks does not stop the others.
pkg() {
  for name in "$@"; do
    if have apt-get; then
      DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "$name" >/tmp/pkg.log 2>&1 || tail -n 5 /tmp/pkg.log
    elif have dnf; then
      dnf install -y "$name" >/tmp/pkg.log 2>&1 || tail -n 5 /tmp/pkg.log
    elif have yum; then
      yum install -y "$name" >/tmp/pkg.log 2>&1 || tail -n 5 /tmp/pkg.log
    elif have zypper; then
      zypper --non-interactive install "$name" >/tmp/pkg.log 2>&1 || tail -n 5 /tmp/pkg.log
    elif have pacman; then
      pacman -S --needed --noconfirm "$name" >/tmp/pkg.log 2>&1 || tail -n 5 /tmp/pkg.log
    elif have xbps-install; then
      xbps-install -y "$name" >/tmp/pkg.log 2>&1 || tail -n 5 /tmp/pkg.log
    fi
  done
}
panel_running() {
  for exe in /proc/[0-9]*/exe; do
    case "$(readlink "$exe" 2>/dev/null || true)" in "$BIN"*) return 0 ;; esac
  done
  return 1
}

step "What a freshly installed desktop already has: a downloader, tar and gzip"
if have apt-get; then apt-get update >/tmp/pkg.log 2>&1 || tail -n 8 /tmp/pkg.log; fi
if have pacman; then pacman -Syu --noconfirm >/dev/null 2>&1; fi
if have xbps-install; then
  xbps-install -Syu -y xbps >/dev/null 2>&1 || true
  xbps-install -Syu -y >/dev/null 2>&1 || true
fi
case "$FETCH" in
  curl) have curl || pkg curl ;;
  wget) have wget || pkg wget ;;
  *) have curl || have wget || pkg curl ;;
esac
pkg ca-certificates
have tar || pkg tar
have gzip || pkg gzip
echo "curl: $(command -v curl || echo no), wget: $(command -v wget || echo no), system: $(sed -n 's/^PRETTY_NAME=//p' /etc/os-release)"
echo "C library: $(ldd --version 2>&1 | head -n1)"

step "This system must be bare: no drawing library may exist yet"
for lib in libX11.so.6 libxkbcommon.so.0 libGL.so.1 libEGL.so.1 libwayland-client.so.0; do
  if find /lib /lib64 /usr/lib /usr/lib64 -name "$lib" 2>/dev/null | grep -q .; then
    fail "$lib is already on this image, so this would not prove a clean install"
  fi
done
echo "bare: confirmed"

step "The one-paste command, as written in the README, without a screen"
set +e
sh -c "$COMMAND" >"$LOG" 2>&1
CODE=$?
set -e
cat "$LOG"
[ "$CODE" = 0 ] || fail "the command ended with code $CODE"
grep -q 'missing: ' "$LOG" || fail "the installer did not notice the missing libraries"
[ -x "$BIN" ] || fail "$BIN was not installed"
LEFT=$("$BIN" doctor --libs)
[ -z "$LEFT" ] || fail "libraries still missing after the install: $LEFT"
echo "every library the program loads is installed now"

step "Test tooling: a virtual screen (installed only now, after the proof above)"
for name in xvfb xorg-x11-server-Xvfb xorg-server-xvfb xorg-x11-server-extra; do
  have Xvfb || pkg "$name" >/dev/null
done
if ! have Xvfb && have dnf; then
  # RHEL 10 and its relatives ship no X server; the extra repositories may, else Wayland is used.
  pkg epel-release >/dev/null
  crb enable >/dev/null 2>&1 || true
  pkg xorg-x11-server-Xvfb >/dev/null
  have Xvfb || pkg weston >/dev/null
fi
if have Xvfb; then
  Xvfb :99 -screen 0 1280x800x24 >/tmp/screen.log 2>&1 &
  sleep 3
  export DISPLAY=:99
  echo "virtual screen: X11 (Xvfb)"
elif have weston; then
  XDG_RUNTIME_DIR=/tmp/xdg-runtime
  export XDG_RUNTIME_DIR
  mkdir -p "$XDG_RUNTIME_DIR" && chmod 700 "$XDG_RUNTIME_DIR"
  weston --backend=headless --socket=sq-test --width=1280 --height=800 >/tmp/screen.log 2>&1 &
  sleep 5
  [ -e "$XDG_RUNTIME_DIR/sq-test" ] || { cat /tmp/screen.log; fail "the virtual Wayland screen did not start"; }
  export WAYLAND_DISPLAY=sq-test
  echo "virtual screen: Wayland (weston)"
else
  fail "could not install a virtual screen (Xvfb or weston) on this image to show the panel on"
fi

step "The same command again, now with a screen: it must start the panel and confirm it"
set +e
sh -c "$COMMAND" >"$LOG" 2>&1
CODE=$?
set -e
cat "$LOG"
[ "$CODE" = 0 ] || fail "the command ended with code $CODE"
grep -q 'The panel is on your desktop now' "$LOG" || fail "the installer did not confirm that the panel started"
sleep 5
panel_running || fail "no Status Quo process is running after the install"
echo "the panel process is running"

step "The command a third time, over the running panel: an update must leave a panel running"
set +e
sh -c "$COMMAND" >"$LOG" 2>&1
CODE=$?
set -e
tail -n 4 "$LOG"
[ "$CODE" = 0 ] || { cat "$LOG"; fail "the update ended with code $CODE"; }
sleep 5
panel_running || fail "no Status Quo process is running after the update"
echo "the panel process is running after the update"

step "The panel draws: a picture of it is saved"
"$BIN" doctor || fail "the compatibility check reports a blocking problem"
"$BIN" stop || fail "stop failed"
panel_running && fail "the panel is still running after stop"
timeout 60 "$BIN" --screenshot "$OUT/panel.png" >/tmp/shot.log 2>&1 || { cat /tmp/shot.log; fail "the panel could not draw a picture of itself"; }
[ "$(wc -c <"$OUT/panel.png")" -gt 2000 ] || fail "the picture of the panel is empty"
echo "picture: $(wc -c <"$OUT/panel.png") bytes"

step "Uninstall"
"$BIN" uninstall || fail "uninstall failed"
sleep 2
[ ! -e "$BIN" ] || fail "the program is still there after uninstall"
panel_running && fail "the panel is still running after uninstall"

printf '\nPROVEN on %s\n' "$(sed -n 's/^PRETTY_NAME=//p' /etc/os-release)"
