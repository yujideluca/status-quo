#!/bin/sh
# Real install test on a full Ubuntu machine with an ordinary user (SQ-47): the path most
# people take. Unlike the bare containers, this user is not the administrator and has systemd,
# so the installer must ask for administrator rights itself (sudo) and the panel must run as
# the user's login service. Nothing is simulated.
#
#   sh linux-host.sh COMMAND [OUT_DIR]
set -eu
COMMAND=$1
OUT=${2:-/tmp}
LOG=/tmp/install.log
BIN=$HOME/.local/bin/status-quo
fail() {
  printf '\nFAILED: %s\n' "$*"
  systemctl --user status status-quo-panel.service 2>&1 | tail -n 30 || true
  exit 1
}
step() { printf '\n######## %s\n' "$*"; }

step "An ordinary user with a login session and a virtual screen"
[ "$(id -u)" != 0 ] || fail "this test must run as an ordinary user"
sudo apt-get update >/dev/null
sudo apt-get install -y xvfb >/dev/null
# Take two of the program's libraries away, so the installer has real work to do.
sudo apt-get remove -y libxkbcommon-x11-0 libxcursor1 >/dev/null 2>&1 || true
sudo loginctl enable-linger "$(id -un)"
XDG_RUNTIME_DIR=/run/user/$(id -u)
export XDG_RUNTIME_DIR
for _ in 1 2 3 4 5 6 7 8 9 10; do
  systemctl --user show-environment >/dev/null 2>&1 && break
  sleep 1
done
systemctl --user show-environment >/dev/null 2>&1 || fail "this machine gives the user no systemd session"
Xvfb :99 -screen 0 1280x800x24 >/tmp/xvfb.log 2>&1 &
sleep 3
export DISPLAY=:99
unset WAYLAND_DISPLAY || true

step "The one-paste command, as written in the README"
set +e
sh -c "$COMMAND" >"$LOG" 2>&1
CODE=$?
set -e
cat "$LOG"
[ "$CODE" = 0 ] || fail "the command ended with code $CODE"
grep -q 'missing: ' "$LOG" || fail "the installer did not notice the removed libraries"
grep -q 'The panel is on your desktop now' "$LOG" || fail "the installer did not confirm that the panel started"
[ -z "$("$BIN" doctor --libs)" ] || fail "libraries still missing: $("$BIN" doctor --libs)"

step "The panel runs as the user's login service and stays up"
sleep 10
for unit in status-quo-collector.service status-quo-panel.service; do
  [ "$(systemctl --user is-active "$unit")" = active ] || fail "$unit is not running"
done
[ -f "$HOME/.config/autostart/status-quo.desktop" ] || fail "nothing starts the panel at login"
"$BIN" doctor || fail "the compatibility check reports a blocking problem"

step "The panel draws: a picture of it is saved"
"$BIN" stop
[ "$(systemctl --user is-active status-quo-panel.service || true)" != active ] || fail "the panel did not stop"
timeout 60 "$BIN" --screenshot "$OUT/panel.png" || fail "the panel could not draw a picture of itself"
[ "$(wc -c <"$OUT/panel.png")" -gt 2000 ] || fail "the picture of the panel is empty"

step "Uninstall"
"$BIN" uninstall || fail "uninstall failed"
[ ! -e "$BIN" ] || fail "the program is still there after uninstall"
[ "$(systemctl --user is-active status-quo-panel.service || true)" != active ] || fail "the panel is still running after uninstall"

printf '\nPROVEN on %s with an ordinary user, sudo and systemd\n' "$(sed -n 's/^PRETTY_NAME=//p' /etc/os-release)"
