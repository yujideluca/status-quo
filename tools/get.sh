#!/bin/sh
# Status Quo, one-paste installer for Linux (SQ-46, SQ-47). Paste this one line in a terminal:
#
#   { curl -fsSL https://raw.githubusercontent.com/yujideluca/status-quo/main/tools/get.sh 2>/dev/null || wget -qO- https://raw.githubusercontent.com/yujideluca/status-quo/main/tools/get.sh; } | sh
#
# It downloads the latest release package for this machine, checks its SHA-256 checksum and runs
# the installer inside it. It needs nothing installed beforehand except one of curl or wget
# (every desktop Linux has one) and works in any POSIX shell (sh, dash, bash, busybox).
# Only when no prebuilt package fits the machine (a processor other than x86-64 and ARM64, or a
# system built on musl instead of glibc) it falls back to building from source with
# tools/install.sh. Options such as --dry-run, --uninstall, --no-launch pass through.
# For tests: SQ_RELEASE_BASE and SQ_RAW_BASE replace the two download addresses.
set -eu

REPO=yujideluca/status-quo
RELEASE_BASE=${SQ_RELEASE_BASE:-https://github.com/$REPO/releases/latest/download}
RAW_BASE=${SQ_RAW_BASE:-https://raw.githubusercontent.com/$REPO/main/tools}

say() { printf '\n==> %s\n' "$*"; }
ok() { printf '  [ok]  %s\n' "$*"; }
warn() { printf '  [!]   %s\n' "$*"; }
die() {
  printf '  [x]   %s\n' "$*" >&2
  exit 1
}

# fetch URL FILE: three attempts, so one dropped connection does not end the installation.
fetch() {
  for attempt in 1 2 3; do
    if fetch_once "$1" "$2"; then return 0; fi
    [ "$attempt" = 3 ] || sleep 2
  done
  return 1
}
# Whether the address answers "not found" (as opposed to the network being down).
not_found() {
  case "$1" in
    file://*)
      [ ! -e "${1#file://}" ]
      return
      ;;
  esac
  [ "$(http_code "$1")" = 404 ]
}
sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d' ' -f1
  elif command -v openssl >/dev/null 2>&1; then
    openssl dgst -sha256 "$1" | sed 's/^.*= *//'
  else
    echo unavailable
  fi
}
unpack() { # unpack ARCHIVE DIR
  if command -v tar >/dev/null 2>&1 && command -v gzip >/dev/null 2>&1; then
    tar -xzf "$1" -C "$2"
  elif command -v python3 >/dev/null 2>&1; then
    python3 -c 'import sys, tarfile; tarfile.open(sys.argv[1]).extractall(sys.argv[2])' "$1" "$2"
  else
    die "this system has neither tar with gzip nor python3 to unpack the download. Install tar and gzip with your package manager and paste the command again."
  fi
}
# musl systems (Alpine, Void musl) cannot run a program built for glibc, the C library of
# every mainstream distribution.
is_musl() {
  for loader in /lib/ld-musl-*.so.1; do
    [ -e "$loader" ] || continue
    ls /lib64/ld-linux-*.so.* /lib/ld-linux-*.so.* /lib/*-linux-gnu/ld-linux-*.so.* >/dev/null 2>&1 && return 1
    return 0
  done
  return 1
}
source_installer() { # source_installer REASON ARGS...
  reason=$1
  shift
  warn "no ready-made package for this machine ($reason): building Status Quo here instead (about ten minutes)"
  fetch "$RAW_BASE/install.sh" "$WORK/install.sh" || die "could not download the source installer from $RAW_BASE. Check the internet connection and paste the command again."
  command -v bash >/dev/null 2>&1 || die "building from source needs bash. Install bash with your package manager and paste the command again."
  exec bash "$WORK/install.sh" --yes "$@"
}

main() {
  [ "$(uname -s)" = Linux ] || die "This command is for Linux. On Windows use the PowerShell line in the README."
  if command -v curl >/dev/null 2>&1; then
    fetch_once() { curl -fsSL "$1" -o "$2"; }
    http_code() { curl -sIL -o /dev/null -w '%{http_code}' "$1" 2>/dev/null || true; }
  elif command -v wget >/dev/null 2>&1; then
    fetch_once() { wget -q "$1" -O "$2"; }
    http_code() { wget --spider -S "$1" 2>&1 | sed -n 's/^ *HTTP\/[0-9.]* \([0-9]*\).*/\1/p' | tail -n1; }
  else
    die "curl or wget is needed to download Status Quo, and this system has neither. Install one with your package manager (for example: sudo apt-get install -y wget) and paste the command again."
  fi

  WORK=$(mktemp -d 2>/dev/null || mktemp -d -t status-quo)
  trap 'rm -rf "$WORK"' EXIT

  say "Status Quo: download"
  case "$(uname -m)" in
    x86_64 | amd64) ARCH=x86_64 ;;
    aarch64 | arm64) ARCH=aarch64 ;;
    *) source_installer "processor $(uname -m)" "$@" ;;
  esac
  if is_musl; then
    source_installer "a musl-based system" "$@"
  fi
  ASSET=status-quo-linux-$ARCH.tar.gz
  if ! fetch "$RELEASE_BASE/$ASSET" "$WORK/$ASSET" 2>/dev/null; then
    if not_found "$RELEASE_BASE/$ASSET"; then
      source_installer "the release has no $ASSET" "$@"
    fi
    die "could not download $ASSET. Check the internet connection and paste the command again. Nothing was installed."
  fi
  fetch "$RELEASE_BASE/$ASSET.sha256" "$WORK/$ASSET.sha256" 2>/dev/null \
    || die "could not download the checksum of $ASSET. Check the internet connection and paste the command again. Nothing was installed."
  ok "downloaded $ASSET"

  expected=$(cut -d' ' -f1 <"$WORK/$ASSET.sha256")
  actual=$(sha256_of "$WORK/$ASSET")
  if [ "$actual" = unavailable ]; then
    warn "no checksum tool on this system (sha256sum, shasum, openssl): the download is not verified"
  elif [ "$expected" != "$actual" ]; then
    die "checksum does not match: the download is damaged or was changed. Nothing was installed. Paste the command again."
  else
    ok "checksum ok"
  fi

  unpack "$WORK/$ASSET" "$WORK"
  [ -f "$WORK/status-quo-linux-$ARCH/install.sh" ] || die "the package has no install.sh inside"
  sh "$WORK/status-quo-linux-$ARCH/install.sh" "$@"
}
# The whole file is read before anything runs: a download cut in the middle does nothing.
main "$@"
