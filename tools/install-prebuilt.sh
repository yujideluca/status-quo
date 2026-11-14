#!/usr/bin/env bash
# Status Quo, prebuilt Linux package (SQ-45). This file ships as install.sh inside
# status-quo-linux-x86_64.tar.gz, next to the ready-made `status-quo` program. No Rust, compiler
# or source is needed. The drawing libraries the program loads when it starts (X11, Wayland,
# OpenGL) are installed here with the system's package manager when missing: sudo asks for the
# password once, and only in that case.
#
#   bash install.sh              install for your user only (no sudo), start it, check the computer
#   bash install.sh --no-deps    never use sudo; you install the system libraries yourself
#   bash install.sh --dry-run    print every step instead of doing it
#   bash install.sh --no-launch  install but do not start it now
#   bash install.sh --adopt      take over a program copied there by hand (a backup is kept)
#   bash install.sh --uninstall  remove Status Quo (settings and history stay)
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
BIN="$HERE/status-quo"
DRY=0 LAUNCH=1 UNINSTALL=0 ADOPT='' DEPS=1
OS_RELEASE=${SQ_OS_RELEASE:-/etc/os-release}

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
    --no-deps) DEPS=0 ;;
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

# --- system libraries the program loads at start ------------------------------------------
os_field() { [ -r "$OS_RELEASE" ] && sed -n "s/^$1=//p" "$OS_RELEASE" | head -n1 | tr -d '"' || true; }
package_manager() {
  case " $(os_field ID) $(os_field ID_LIKE) " in
    *" debian "* | *" ubuntu "*) echo apt ;;
    *" fedora "* | *" rhel "* | *" centos "*) echo dnf ;;
    *" arch "*) echo pacman ;;
    *" suse "* | *" opensuse "* | *" opensuse-leap "* | *" opensuse-tumbleweed "*) echo zypper ;;
    *) echo unknown ;;
  esac
}
runtime_packages() {
  case "$1" in
    apt) echo "libx11-6 libxcb1 libxkbcommon0 libxkbcommon-x11-0 libwayland-client0 libgl1 libegl1 libxcursor1 libxi6 libxrandr2" ;;
    dnf) echo "libX11 libxcb libxkbcommon libxkbcommon-x11 libwayland-client mesa-libGL mesa-libEGL libXcursor libXi libXrandr" ;;
    pacman) echo "libx11 libxcb libxkbcommon libxkbcommon-x11 wayland mesa libxcursor libxi libxrandr" ;;
    zypper) echo "libX11-6 libxcb1 libxkbcommon0 libxkbcommon-x11-0 libwayland-client0 Mesa-libGL1 Mesa-libEGL1 libXcursor1 libXi6 libXrandr2" ;;
  esac
}
install_command() {
  case "$1" in
    apt) echo "apt-get install -y" ;;
    dnf) echo "dnf install -y" ;;
    pacman) echo "pacman -S --needed --noconfirm" ;;
    zypper) echo "zypper --non-interactive install" ;;
  esac
}
libs_missing() {
  # For tests: SQ_ASSUME_MISSING_LIBS=1 pretends they are missing, =0 pretends they are present.
  [ "${SQ_ASSUME_MISSING_LIBS:-}" = 1 ] && return 0
  [ "${SQ_ASSUME_MISSING_LIBS:-}" = 0 ] && return 1
  local list lib
  if [ -n "${SQ_LDCONFIG_LIST:-}" ]; then # for tests: a file standing in for `ldconfig -p`
    list=$(cat "$SQ_LDCONFIG_LIST")
  else
    # ldconfig lives in /sbin, which a normal user's PATH often lacks.
    list=$( (ldconfig -p || /sbin/ldconfig -p || /usr/sbin/ldconfig -p) 2>/dev/null || true)
    [ -n "$list" ] || return 1
  fi
  for lib in libX11.so.6 libxcb.so.1 libxkbcommon.so.0 libxkbcommon-x11.so.0 libwayland-client.so.0 libGL.so.1 libEGL.so.1 libXcursor.so.1 libXi.so.6 libXrandr.so.2; do
    # A here-string, not a pipe: under pipefail grep -q closing the pipe early reads as a failure.
    grep -q "$lib" <<<"$list" || return 0
  done
  return 1
}
say "System libraries"
if [ "$DEPS" = 0 ]; then
  warn "skipped (--no-deps)"
elif ! libs_missing; then
  ok "already installed"
else
  PM=$(package_manager)
  [ "$PM" != unknown ] || die "unknown distribution: install the X11, Wayland, xkbcommon and OpenGL libraries with your package manager, then run again with --no-deps."
  PKGS=$(runtime_packages "$PM")
  echo "  installing with $PM: $PKGS (your password may be asked once)"
  SUDO=""
  [ "$(id -u)" = 0 ] || SUDO="sudo"
  [ "$PM" = apt ] && run $SUDO apt-get update
  # shellcheck disable=SC2046,SC2086
  run $SUDO $(install_command "$PM") $PKGS
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
