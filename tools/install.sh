#!/usr/bin/env bash
# Status Quo installer for Linux: checks the computer, installs what is missing, builds the
# program, installs it for the current user and runs the compatibility check (SQ-41).
#
#   bash tools/install.sh              full installation
#   bash tools/install.sh --check-only look only; change nothing
#   bash tools/install.sh --dry-run    print every step instead of doing it
#   bash tools/install.sh --uninstall  remove Status Quo (settings and history stay)
# Other options: --yes (do not ask), --no-deps (never use sudo), --repo URL, --src DIR.
set -euo pipefail

MIN_RUST_MAJOR=1
MIN_RUST_MINOR=87
REPO_URL=${SQ_REPO_URL:-https://github.com/yujideluca/status-quo.git}
SRC_DIR=${SQ_SRC:-}
OS_RELEASE=${SQ_OS_RELEASE:-/etc/os-release}
DRY=0 YES=0 DEPS=1 CHECK=0 UNINSTALL=0

say() { printf '\n==> %s\n' "$*"; }
ok() { printf '  [ok]  %s\n' "$*"; }
warn() { printf '  [!]   %s\n' "$*"; }
die() { printf '  [x]   %s\n' "$*" >&2; exit 1; }
# Print the command in a dry run, run it otherwise.
run() {
  if [ "$DRY" = 1 ]; then
    printf '  + %s\n' "$*"
  else
    "$@"
  fi
}

while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) DRY=1 ;;
    --yes | -y) YES=1 ;;
    --no-deps) DEPS=0 ;;
    --check-only) CHECK=1 ;;
    --uninstall) UNINSTALL=1 ;;
    --repo) shift; REPO_URL=${1:?--repo needs a URL} ;;
    --src) shift; SRC_DIR=${1:?--src needs a folder} ;;
    -h | --help) sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "unknown option: $1 (try --help)" ;;
  esac
  shift
done

# --- 1. what computer is this -------------------------------------------------------------
os_field() { # os_field ID -> value of ID= in os-release, empty when absent
  [ -r "$OS_RELEASE" ] || return 0
  sed -n "s/^$1=//p" "$OS_RELEASE" | head -n1 | tr -d '"'
}
package_manager() {
  local id like
  id=$(os_field ID)
  like=$(os_field ID_LIKE)
  case " $id $like " in
    *" debian "* | *" ubuntu "*) echo apt ;;
    *" fedora "* | *" rhel "* | *" centos "*) echo dnf ;;
    *" arch "*) echo pacman ;;
    *" suse "* | *" opensuse "* | *" opensuse-leap "* | *" opensuse-tumbleweed "*) echo zypper ;;
    *) echo unknown ;;
  esac
}
# A compiler, git, curl, pkg-config and the development files of the libraries Status Quo draws
# with (X11, Wayland, keyboard maps, OpenGL). Debian/Ubuntu names are the ones the project's CI
# installs; the others are the usual names and are untested.
packages_for() {
  case "$1" in
    apt) echo "build-essential pkg-config libx11-dev libxkbcommon-dev libwayland-dev libgl1-mesa-dev git curl ca-certificates" ;;
    dnf) echo "gcc pkgconf-pkg-config libX11-devel libxkbcommon-devel wayland-devel mesa-libGL-devel git curl" ;;
    pacman) echo "base-devel pkgconf libx11 libxkbcommon wayland mesa git curl" ;;
    zypper) echo "gcc pkg-config libX11-devel libxkbcommon-devel wayland-devel Mesa-libGL-devel git curl" ;;
    *) echo "" ;;
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
deps_missing() {
  [ "${SQ_ASSUME_MISSING_DEPS:-0}" = 1 ] && return 0
  command -v cc >/dev/null 2>&1 || return 0
  command -v git >/dev/null 2>&1 || return 0
  command -v curl >/dev/null 2>&1 || return 0
  command -v pkg-config >/dev/null 2>&1 || return 0
  # The development files of X11, xkbcommon, Wayland and OpenGL are not required to build (the
  # program loads them when it starts), so their absence alone does not trigger an install.
  return 1
}
rust_ok() {
  [ "${SQ_ASSUME_NO_RUST:-0}" = 1 ] && return 1
  command -v cargo >/dev/null 2>&1 && command -v rustc >/dev/null 2>&1 || return 1
  local v major minor
  v=$(rustc --version | awk '{print $2}')
  major=${v%%.*}
  minor=${v#*.}
  minor=${minor%%.*}
  [ "$major" -gt "$MIN_RUST_MAJOR" ] || { [ "$major" -eq "$MIN_RUST_MAJOR" ] && [ "$minor" -ge "$MIN_RUST_MINOR" ]; }
}
confirm() { # confirm "question": yes when --yes, or when the user answers y
  [ "$YES" = 1 ] && return 0
  [ "$DRY" = 1 ] && return 0
  if [ -r /dev/tty ]; then
    local reply
    printf '  %s [Y/n] ' "$1" >/dev/tty
    read -r reply </dev/tty || reply=n
    case "$reply" in "" | y | Y | yes) return 0 ;; esac
    return 1
  fi
  die "$1 Cannot ask (no terminal). Run again with --yes."
}

say "1/6 Checking this computer"
[ "$(uname -s)" = Linux ] || die "This installer is for Linux. On Windows use status-quo-setup.exe (see the install guide)."
ok "kernel: $(uname -sr), $(uname -m)"
case "$(uname -m)" in x86_64 | aarch64) ;; *) warn "architecture $(uname -m) is untested" ;; esac
PM=$(package_manager)
ok "distribution: $(os_field PRETTY_NAME) (package manager: $PM)"
SESSION=${XDG_SESSION_TYPE:-unknown}
if [ -n "${WAYLAND_DISPLAY:-}" ] || [ -n "${DISPLAY:-}" ]; then
  ok "graphical session: $SESSION, desktop: ${XDG_CURRENT_DESKTOP:-unknown}"
else
  warn "no graphical session in this terminal (WAYLAND_DISPLAY and DISPLAY are empty); building works, starting the panel needs your desktop"
fi
optional_tool() { # optional_tool NAME "what is lost without it"
  if command -v "$1" >/dev/null 2>&1; then ok "$1 found"; else warn "$1 not installed: $2"; fi
}
optional_tool gh "the GitHub widget will show unavailable (https://cli.github.com)"
optional_tool claude "Claude widgets will show unavailable"
optional_tool codex "Codex widgets will show unavailable"

find_source() {
  local here
  here=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." 2>/dev/null && pwd || true)
  if [ -z "$SRC_DIR" ] && [ -n "$here" ] && grep -q 'crates/desktop' "$here/Cargo.toml" 2>/dev/null; then
    SRC_DIR=$here
  fi
  SRC_DIR=${SRC_DIR:-$HOME/.local/share/status-quo/src}
}
find_source
BIN="$SRC_DIR/target/release/status-quo"

if [ "$UNINSTALL" = 1 ]; then
  say "Uninstalling"
  if [ -x "$BIN" ]; then run "$BIN" uninstall
  elif command -v status-quo >/dev/null 2>&1; then run status-quo uninstall
  else die "no status-quo program found to uninstall with"; fi
  exit 0
fi

# The doctor of a program built before it existed would start the panel instead: ask first.
run_doctor() {
  local bin=$1
  if "$bin" --help 2>&1 | grep -q 'status-quo doctor'; then
    "$bin" doctor || return 0
  else
    warn "the installed Status Quo is older and has no doctor command; run the installer to update it"
  fi
}

if [ "$CHECK" = 1 ]; then
  say "Compatibility check (nothing is changed)"
  if deps_missing; then warn "build libraries missing: the installer would install ($PM): $(packages_for "$PM")"; else ok "build libraries present"; fi
  if rust_ok; then ok "Rust $(rustc --version | awk '{print $2}')"; else warn "Rust ${MIN_RUST_MAJOR}.${MIN_RUST_MINOR} or newer missing: the installer would install it with rustup"; fi
  if [ -x "$BIN" ]; then run_doctor "$BIN"
  elif command -v status-quo >/dev/null 2>&1; then run_doctor "$(command -v status-quo)"
  else warn "Status Quo itself is not installed yet; run without --check-only to install it"; fi
  exit 0
fi

# --- 2. build libraries -------------------------------------------------------------------
say "2/6 Build libraries"
if [ "$DEPS" = 0 ]; then
  warn "skipped (--no-deps)"
elif ! deps_missing; then
  ok "already installed"
elif [ "$PM" = unknown ]; then
  die "unknown distribution. Install a C compiler, git, curl, pkg-config and the development files of X11, xkbcommon, Wayland and OpenGL, then run again with --no-deps."
else
  PKGS=$(packages_for "$PM")
  echo "  needs: $PKGS"
  confirm "Install these with sudo?" || die "cannot continue without them (or run again with --no-deps once they are installed)"
  SUDO=""
  [ "$(id -u)" = 0 ] || SUDO="sudo"
  [ "$PM" = apt ] && run $SUDO apt-get update
  # shellcheck disable=SC2046,SC2086
  run $SUDO $(install_command "$PM") $PKGS
fi

# --- 3. Rust ------------------------------------------------------------------------------
say "3/6 Rust ${MIN_RUST_MAJOR}.${MIN_RUST_MINOR} or newer"
# shellcheck source=/dev/null
[ -r "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
if rust_ok; then
  ok "found $(rustc --version)"
else
  confirm "Install Rust with rustup (into your home folder, no sudo)?" || die "Rust is required"
  if [ "$DRY" = 1 ]; then
    echo "  + curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal"
  else
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
    # shellcheck source=/dev/null
    . "$HOME/.cargo/env"
  fi
fi

# --- 4. source ----------------------------------------------------------------------------
say "4/6 Source code"
if [ -d "$SRC_DIR/.git" ] || [ -f "$SRC_DIR/Cargo.toml" ]; then
  ok "using $SRC_DIR"
  if [ -d "$SRC_DIR/.git" ] && [ "$SRC_DIR" = "$HOME/.local/share/status-quo/src" ]; then
    run git -C "$SRC_DIR" pull --ff-only
  fi
else
  run mkdir -p "$(dirname "$SRC_DIR")"
  run git clone "$REPO_URL" "$SRC_DIR"
fi

# --- 5. build and install -----------------------------------------------------------------
say "5/6 Building (a few minutes the first time)"
if [ "$DRY" = 1 ]; then
  echo "  + (cd $SRC_DIR && cargo build --release --locked -p status-quo)"
else
  if ! (cd "$SRC_DIR" && cargo build --release --locked -p status-quo); then
    warn "the build failed. If the messages above name a missing library, install: $(packages_for "$PM")"
    exit 1
  fi
fi
say "Installing for your user (no sudo)"
run "$BIN" install
if [ -n "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ]; then
  run "$BIN" launch || warn "could not start it now; it starts at your next login"
fi

# --- 6. compatibility report --------------------------------------------------------------
say "6/6 Compatibility check"
if [ "$DRY" = 1 ]; then
  echo "  + $BIN doctor"
else
  "$BIN" doctor || warn "the check found a blocking problem; read the lines marked [x]"
fi
say "Done. Uninstall any time with: bash tools/install.sh --uninstall"
