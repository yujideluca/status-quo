#!/bin/sh
# Real self-update test on Linux (SQ-54). Runs as root inside a bare container (the panel is an
# ordinary background program there), or as an ordinary user with sudo on a full Ubuntu, where
# the panel is a systemd user service and the update must survive the service's restart. The previous
# release is installed with the README line, its real panel is started on a virtual screen, and
# the test waits until that panel has found the newest published release on GitHub, checked it
# and installed it by itself. Nothing is simulated: only the wait before the first check is
# shortened (STATUS_QUO_UPDATE_DELAY), the download, checksum and installation are the real ones.
#
#   sh linux-update.sh COMMAND FROM_TAG TO_TAG [OUT_DIR]
#
# COMMAND is the install line of the README; FROM_TAG the release installed first; TO_TAG the
# release it must update itself to. A test that passes prints "PROVEN" as its last line.
set -eu
COMMAND=$1
FROM=$2
TO=$3
OUT=${4:-/tmp}
BIN=$HOME/.local/bin/status-quo
MANIFEST=$HOME/.local/share/status-quo/install.json
STATE=$HOME/.local/state/status-quo

step() { printf '\n######## %s\n' "$*"; }
fail() {
  printf '\nFAILED: %s\n' "$*"
  for f in "$STATE/update.log" "$STATE/panel.log"; do
    [ -f "$f" ] && { echo "--- $f"; tail -n 40 "$f"; }
  done
  exit 1
}
installed() { sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' "$MANIFEST" 2>/dev/null; }

step "A downloader and a virtual screen"
export DEBIAN_FRONTEND=noninteractive
SUDO=
[ "$(id -u)" = 0 ] || SUDO=sudo
$SUDO apt-get update >/tmp/pkg.log 2>&1 || tail -n 8 /tmp/pkg.log
$SUDO apt-get install -y --no-install-recommends curl ca-certificates xvfb >>/tmp/pkg.log 2>&1 || {
  tail -n 8 /tmp/pkg.log
  fail "the test tools could not be installed"
}
SYSTEMD=no
if [ -n "$SUDO" ]; then
  sudo loginctl enable-linger "$(id -un)"
  XDG_RUNTIME_DIR=/run/user/$(id -u)
  export XDG_RUNTIME_DIR
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    systemctl --user show-environment >/dev/null 2>&1 && break
    sleep 1
  done
  systemctl --user show-environment >/dev/null 2>&1 || fail "this machine gives the user no systemd session"
  SYSTEMD=yes
fi
echo "panel as a systemd user service: $SYSTEMD"

step "Install $FROM with the README line"
SQ_RELEASE_BASE="https://github.com/yujideluca/status-quo/releases/download/$FROM" sh -c "$COMMAND" >/tmp/install.log 2>&1 || {
  cat /tmp/install.log
  fail "installing $FROM failed"
}
[ "$(installed)" = "${FROM#v}" ] || fail "install.json names $(installed), not ${FROM#v}"
echo "installed: $(installed)"

step "Start the installed panel and wait for it to update itself to $TO"
Xvfb :99 -screen 0 1280x800x24 >/tmp/screen.log 2>&1 &
sleep 3
export DISPLAY=:99 STATUS_QUO_UPDATE_DELAY=20
# A user service gets its environment from the service manager, not from this shell.
[ "$SYSTEMD" = no ] || systemctl --user set-environment STATUS_QUO_UPDATE_DELAY=20
"$BIN" launch --verify || fail "the panel of $FROM did not start"
i=0
while [ "$(installed)" != "${TO#v}" ]; do
  i=$((i + 1))
  [ $i -le 120 ] || fail "after 10 minutes install.json still names $(installed), not ${TO#v}"
  sleep 5
done
echo "install.json names ${TO#v} after about $((i * 5)) seconds"

step "The new version runs"
sleep 10
running=no
for exe in /proc/[0-9]*/exe; do
  case "$(readlink "$exe" 2>/dev/null || true)" in "$BIN") running=yes ;; esac
done
[ "$running" = yes ] || fail "no Status Quo process runs after the update"
if [ "$SYSTEMD" = yes ]; then
  for unit in status-quo-collector.service status-quo-panel.service; do
    [ "$(systemctl --user is-active "$unit")" = active ] || fail "$unit is not running after the update"
  done
  echo "both user services run"
fi
"$BIN" update --check | tee /tmp/check.log
grep -q "is the newest version" /tmp/check.log || fail "the updated program does not see itself as the newest"
cp "$STATE/update.log" "$OUT/update.log" 2>/dev/null || true
echo "--- update.log"
cat "$STATE/update.log"
echo
echo "PROVEN"
