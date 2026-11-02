#!/usr/bin/env bash
# Desktop end-to-end test of the real tray (SQ-15, SQ-16, SQ-27, SQ-28, SQ-29). Run it before closing any
# major tray update. It pauses the live tray, drives a private copy with a virtual mouse made through
# /dev/uinput (it needs the screen to itself: see the refusal message), writes screenshots and report.json to build/e2e/, and restarts the live tray afterwards.
set -euo pipefail
cd "$(dirname "$0")/.."
[ -n "${WAYLAND_DISPLAY:-}" ] || { echo "skipped: no Wayland session"; exit 0; }
[ -w /dev/uinput ] || { echo "skipped: /dev/uinput is not writable"; exit 0; }
command -v spectacle >/dev/null || { echo "skipped: spectacle is not installed"; exit 0; }
if [ "${STATUS_QUO_E2E_SCREEN_IS_FREE:-0}" != 1 ]; then
  cat >&2 <<'MSG'
refused: this test moves a virtual mouse and clicks on the screen for about three minutes.
Clicks land on whatever window is in front (it once hit a maximized browser). Close or minimize
every window that covers the right side of the screen, keep your hands off the mouse and keyboard,
then run:  STATUS_QUO_E2E_SCREEN_IS_FREE=1 tools/ui_e2e.sh
MSG
  exit 2
fi
cargo build --release --locked -p status-quo
STATUS_QUO_E2E_BIN="$PWD/target/release/status-quo" \
  cargo test -p status-quo --test desktop_e2e --locked -- --ignored --nocapture
echo "report: build/e2e/report.json, screenshots: build/e2e/*.png"
