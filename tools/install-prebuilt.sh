#!/usr/bin/env bash
# Status Quo, prebuilt Linux package (SQ-45). This file ships as install.sh inside
# status-quo-linux-x86_64.tar.gz, next to the ready-made `status-quo` program. No Rust, compiler
# or source is needed.
#
#   bash install.sh              install for your user only (no sudo), start it, check the computer
#   bash install.sh --dry-run    print every step instead of doing it
#   bash install.sh --no-launch  install but do not start it now
#   bash install.sh --adopt      take over a program copied there by hand (a backup is kept)
#   bash install.sh --uninstall  remove Status Quo (settings and history stay)
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
BIN="$HERE/status-quo"
DRY=0 LAUNCH=1 UNINSTALL=0 ADOPT=

say() { printf '\n==> %s\n' "$*"; }
ok() { printf '  [ok]  %s\n' "$*"; }
warn() { printf '  [!]   %s\n' "$*"; }
die() { printf '  [x]   %s\n' "$*" >&2; exit 1; }
run() {
  if [ "$DRY" = 1 ]; then printf '  + %s\n' "$*"; else "$@"; fi
}

while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) DRY=1 ;;
    --no-launch) LAUNCH=0 ;;
    --adopt) ADOPT=--adopt ;;
    --uninstall) UNINSTALL=1 ;;
    -h | --help) sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "unknown option: $1 (try --help)" ;;
  esac
  shift
done

say "Checking this computer"
[ "$(uname -s)" = Linux ] || die "This package is for Linux. On Windows use status-quo-setup.exe."
[ "$(uname -m)" = x86_64 ] || die "This package is built for x86_64 only (this is $(uname -m)). Build from source: bash tools/install.sh"
[ -x "$BIN" ] || die "the program 'status-quo' is not next to this script ($HERE). Unpack the whole .tar.gz and run install.sh from inside it."
ok "$(uname -sr), $(uname -m)"

if [ "$UNINSTALL" = 1 ]; then
  say "Uninstalling"
  run "$BIN" uninstall
  exit 0
fi

say "Installing for your user only"
run "$BIN" install $ADOPT
if [ "$LAUNCH" = 1 ] && [ -n "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ]; then
  run "$BIN" launch || warn "could not start it now; it starts at your next login"
fi

say "Compatibility check"
if [ "$DRY" = 1 ]; then
  echo "  + $BIN doctor"
else
  "$BIN" doctor || warn "the check found a blocking problem; read the lines marked [x]"
fi
say "Done. Uninstall any time with: bash install.sh --uninstall"
