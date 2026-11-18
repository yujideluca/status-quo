#!/bin/sh
# Status Quo, prebuilt Linux package (SQ-45, SQ-47). This file ships as install.sh inside
# status-quo-linux-<processor>.tar.gz, next to the ready-made `status-quo` program. No Rust,
# compiler or source is needed. The program asks the system for its drawing libraries (X11,
# Wayland, OpenGL) when it starts; this installer asks the program which ones are missing and
# installs the packages that hold them, with the system's package manager. Only then a
# password is asked, once. A library that cannot be installed never stops the installation:
# the program is installed anyway and the final check says in plain words what is left.
#
#   sh install.sh              install for your user only, start it, check the computer
#   sh install.sh --no-deps    never ask for a password; you install the system libraries yourself
#   sh install.sh --dry-run    print every step instead of doing it
#   sh install.sh --no-launch  install but do not start it now
#   sh install.sh --adopt      take over a program copied there by hand (a backup is kept)
#   sh install.sh --uninstall  remove Status Quo (settings and history stay)
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
BIN="$HERE/status-quo"
DRY=0 LAUNCH=1 UNINSTALL=0 ADOPT='' DEPS=1
OS_RELEASE=${SQ_OS_RELEASE:-/etc/os-release}
INSTALLED="$HOME/.local/bin/status-quo"

say() { printf '\n==> %s\n' "$*"; }
ok() { printf '  [ok]  %s\n' "$*"; }
warn() { printf '  [!]   %s\n' "$*"; }
die() {
  printf '  [x]   %s\n' "$*" >&2
  exit 1
}
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
    -h | --help)
      sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) die "unknown option: $1 (try --help)" ;;
  esac
  shift
done

say "Checking this computer"
[ "$(uname -s)" = Linux ] || die "This package is for Linux. On Windows use status-quo-setup.exe."
[ -f "$BIN" ] || die "the program 'status-quo' is not next to this script ($HERE). Unpack the whole .tar.gz and run install.sh from inside it."
[ -x "$BIN" ] || chmod +x "$BIN" 2>/dev/null || true
# The program must be able to run from where it was unpacked. Some systems forbid running
# programs from the temporary folder: a copy in the user's own cache folder is used then.
if ! "$BIN" --help >/dev/null 2>&1; then
  CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/status-quo-setup-$$"
  mkdir -p "$CACHE"
  trap 'rm -rf "$CACHE"' EXIT
  cp "$BIN" "$CACHE/status-quo" && chmod +x "$CACHE/status-quo"
  if "$CACHE/status-quo" --help >/dev/null 2>&1; then
    BIN="$CACHE/status-quo"
  else
    die "this computer cannot run the packaged program (it is built for $(basename "$HERE" | sed 's/status-quo-linux-//') processors with glibc 2.17 or newer; this is $(uname -m)). Build from source instead: see 'Other ways to install' in the README."
  fi
fi
ok "$(uname -sr), $(uname -m)"

if [ "$UNINSTALL" = 1 ]; then
  say "Uninstalling"
  if [ -x "$INSTALLED" ]; then run "$INSTALLED" uninstall; else run "$BIN" uninstall; fi
  exit 0
fi

# --- system libraries the program loads at start ------------------------------------------
os_field() {
  [ -r "$OS_RELEASE" ] || return 0
  sed -n "s/^$1=//p" "$OS_RELEASE" | head -n1 | tr -d '"'
}
# The distribution's family decides; on a distribution this script has never heard of, the
# package manager that is actually installed does.
package_manager() {
  case " $(os_field ID) $(os_field ID_LIKE) " in
    *" debian "* | *" ubuntu "*) echo apt ;;
    *" fedora "* | *" rhel "* | *" centos "*) echo dnf ;;
    *" arch "*) echo pacman ;;
    *" suse "* | *" opensuse "* | *" opensuse-leap "* | *" opensuse-tumbleweed "*) echo zypper ;;
    *" void "*) echo xbps ;;
    *)
      for candidate in apt-get:apt dnf:dnf yum:dnf zypper:zypper pacman:pacman xbps-install:xbps; do
        if command -v "${candidate%%:*}" >/dev/null 2>&1; then
          echo "${candidate##*:}"
          return 0
        fi
      done
      echo unknown
      ;;
  esac
}
# The packages that hold the drawing libraries and a working OpenGL driver. Each list is
# installed for real, on a bare system of its family, by the project's install tests.
runtime_packages() {
  case "$1" in
    apt) echo "libx11-6 libx11-xcb1 libxcb1 libxkbcommon0 libxkbcommon-x11-0 libwayland-client0 libwayland-egl1 libgl1 libegl1 libxcursor1 libxi6 libxrender1" ;;
    dnf) echo "libX11 libX11-xcb libxcb libxkbcommon libxkbcommon-x11 libwayland-client libwayland-egl mesa-libGL mesa-libEGL mesa-dri-drivers libXcursor libXi libXrender" ;;
    pacman) echo "libx11 libxcb libxkbcommon libxkbcommon-x11 wayland mesa libglvnd libxcursor libxi libxrender" ;;
    zypper) echo "libX11-6 libX11-xcb1 libxcb1 libxkbcommon0 libxkbcommon-x11-0 libwayland-client0 libwayland-egl1 Mesa-libGL1 Mesa-libEGL1 Mesa-dri libXcursor1 libXi6 libXrender1" ;;
    xbps) echo "libX11 libxcb libxkbcommon libxkbcommon-x11 wayland libglvnd mesa-dri libXcursor libXi libXrender" ;;
  esac
}
install_command() {
  case "$1" in
    apt) echo "apt-get install -y" ;;
    dnf) if ! command -v dnf >/dev/null 2>&1 && command -v yum >/dev/null 2>&1; then echo "yum install -y"; else echo "dnf install -y"; fi ;;
    pacman) echo "pacman -S --needed --noconfirm" ;;
    zypper) echo "zypper --non-interactive install" ;;
    xbps) echo "xbps-install -Sy" ;;
  esac
}
# Run a command as the administrator: directly when already root, else through whichever of
# sudo, doas or su this system has. The password prompt comes from that tool.
as_root() {
  if [ "$(id -u)" = 0 ]; then
    "$@"
  elif command -v sudo >/dev/null 2>&1; then
    sudo "$@"
  elif command -v doas >/dev/null 2>&1; then
    doas "$@"
  elif command -v su >/dev/null 2>&1; then
    echo "  (the administrator password of this computer, 'root', is asked now)"
    su -c "$*"
  else
    return 1
  fi
}
missing_libraries() { "$BIN" doctor --libs 2>/dev/null | tr '\n' ' ' | sed 's/ *$//'; }
install_libraries() { # install_libraries PM: all at once, then one by one if that fails
  packages=$(runtime_packages "$1")
  command=$(install_command "$1")
  echo "  installing with $1: $packages"
  echo "  (your password may be asked once: these libraries are shared by the whole system)"
  if [ "$DRY" = 1 ]; then
    [ "$1" = apt ] && echo "  + apt-get update"
    printf '  + %s %s\n' "$command" "$packages"
    return 0
  fi
  if [ "$1" = apt ]; then
    as_root apt-get update || warn "the package list could not be refreshed; continuing with the list this system already has"
  fi
  # shellcheck disable=SC2086
  if as_root $command $packages; then return 0; fi
  warn "installing them together failed; trying one package at a time"
  for package in $packages; do
    # shellcheck disable=SC2086
    as_root $command "$package" >/dev/null 2>&1 || warn "could not install $package"
  done
}

say "System libraries"
MISSING=$(missing_libraries)
if [ -z "$MISSING" ]; then
  ok "already installed"
elif [ "$DEPS" = 0 ]; then
  warn "missing: $MISSING (not installed because of --no-deps)"
else
  echo "  missing: $MISSING"
  PM=$(package_manager)
  if [ "$PM" = unknown ]; then
    warn "this distribution's package manager is not one this installer knows; nothing was installed"
  elif [ -e /run/ostree-booted ]; then
    warn "this is an image-based system (such as Fedora Silverblue): system packages are not changed from here"
  else
    install_libraries "$PM"
  fi
  if [ "$DRY" = 0 ]; then
    MISSING=$(missing_libraries)
    if [ -z "$MISSING" ]; then
      ok "installed"
    else
      warn "still missing: $MISSING. Status Quo is installed anyway; the check at the end says whether the panel can open without them."
    fi
  fi
fi

say "Installing for your user only"
# shellcheck disable=SC2086
run "$BIN" install $ADOPT

STARTED=skipped
if [ "$LAUNCH" = 1 ]; then
  say "Starting Status Quo"
  if [ -z "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ]; then
    warn "this terminal is not inside a desktop session, so there is no screen to open the panel on now; it opens at your next login"
  elif [ "$DRY" = 1 ]; then
    echo "  + $INSTALLED launch --verify"
  elif "$INSTALLED" launch --verify; then
    STARTED=yes
  else
    STARTED=no
  fi
fi

say "Compatibility check"
if [ "$DRY" = 1 ]; then
  echo "  + $INSTALLED doctor"
else
  "$INSTALLED" doctor || true
fi

if [ "$STARTED" = no ]; then
  say "Status Quo was installed, but the panel did not start."
  echo "  The lines above say why. After fixing them, start it with:"
  echo "    $INSTALLED launch --verify"
  exit 1
fi
say "Done."
[ "$STARTED" = yes ] && echo "  The panel is on your desktop now and opens by itself at every login."
echo "  Health report any time:  $INSTALLED doctor"
echo "  Remove it any time:      $INSTALLED uninstall"
