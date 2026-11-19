#!/bin/sh
# Real run on a desktop with layer-shell (SQ-26, SQ-48), the way KDE Plasma on Wayland, sway and
# Hyprland place the tray on the desktop layer. Starts sway on a virtual monitor, installs the
# package with its own install.sh, starts the panel and checks from outside that it draws: the
# panel says it uses layer-shell, keeps running, and a screenshot of the monitor shows it.
# Version 0.1.5 stopped at start on these desktops (no graphics backend built in); this test
# would have caught it. Nothing is simulated.
#
#   sh linux-layer-shell.sh PACKAGE.tar.gz
set -eu
PACKAGE=$1
BIN=$HOME/.local/bin/status-quo
STATE=$HOME/.local/state/status-quo
# The panel writes to panel.log when it runs as a plain process, and to the systemd journal when
# it runs as a user service (the GitHub machine has a systemd user session).
panel_output() {
  cat "$STATE/panel.log" 2>/dev/null || true
  journalctl --user -u status-quo-panel.service --no-pager -o cat 2>/dev/null || true
}
fail() {
  printf '\nFAILED: %s\n' "$*"
  echo '--- panel'
  panel_output | tail -n 30
  echo '--- sway'
  tail -n 30 /tmp/sway.log 2>/dev/null || true
  exit 1
}
step() { printf '\n######## %s\n' "$*"; }

step "sway, a layer-shell desktop, on a virtual monitor"
sudo apt-get update >/dev/null
sudo apt-get install -y sway grim >/dev/null
XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/tmp/runtime-$(id -u)}
mkdir -p "$XDG_RUNTIME_DIR" && chmod 700 "$XDG_RUNTIME_DIR"
export XDG_RUNTIME_DIR
unset DISPLAY WAYLAND_DISPLAY || true
printf 'xwayland disable\noutput HEADLESS-1 resolution 1280x800 background #204060 solid_color\n' >/tmp/sway.conf
WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 sway -c /tmp/sway.conf >/tmp/sway.log 2>&1 &
for _ in $(seq 30); do
  socket=$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -name 'wayland-*' ! -name '*.lock' | head -n1)
  [ -n "$socket" ] && break
  sleep 1
done
[ -n "$socket" ] || fail "sway did not start"
WAYLAND_DISPLAY=$(basename "$socket")
export WAYLAND_DISPLAY
echo "sway runs: $WAYLAND_DISPLAY"

step "Install the package and start the panel"
mkdir -p /tmp/pkg && tar -C /tmp/pkg -xzf "$PACKAGE"
sh /tmp/pkg/*/install.sh || fail "install.sh failed"
sleep 10
panel_output >/tmp/panel-output.log
grep -q 'Status Quo host: layer-shell' /tmp/panel-output.log || fail "the panel did not choose layer-shell"
if grep -q 'panicked' /tmp/panel-output.log; then fail "the panel stopped with an error"; fi
running=no
for exe in /proc/[0-9]*/exe; do
  [ "$(readlink "$exe" 2>/dev/null || true)" = "$BIN" ] || continue
  case "$(tr '\0' ' ' <"${exe%/exe}/cmdline")" in *collect*) ;; *) running=yes ;; esac
done
[ "$running" = yes ] || fail "the panel is not running"

step "The panel is drawn on the monitor"
grim -t ppm /tmp/shot.ppm || fail "no screenshot"
python3 - /tmp/shot.ppm <<'EOF' || fail "the screenshot shows only the background"
import sys
data = open(sys.argv[1], "rb").read()
parts = data.split(b"\n", 3)
width, height = map(int, parts[1].split())
pixels = parts[3]
background = (0x20, 0x40, 0x60)
other = sum(1 for i in range(0, width * height * 3, 3) if tuple(pixels[i:i + 3]) != background)
share = other / (width * height)
print(f"pixels that are not the background: {share:.1%}")
sys.exit(0 if share > 0.05 else 1)
EOF
echo
echo "PROVEN"
