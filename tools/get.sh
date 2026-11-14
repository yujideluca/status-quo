#!/usr/bin/env bash
# Status Quo, one-paste installer for Linux (SQ-46):
#
#   curl -fsSL https://raw.githubusercontent.com/yujideluca/status-quo/main/tools/get.sh | bash
#
# Downloads the latest release package for this machine, checks its SHA-256 checksum and runs
# the installer inside it. With no prebuilt package (no release yet, another architecture) it
# falls back to the source installer tools/install.sh. Options such as --dry-run, --uninstall,
# --no-launch pass through to the installer that runs.
# For tests: SQ_RELEASE_BASE and SQ_RAW_BASE replace the two download addresses.
set -euo pipefail

REPO=yujideluca/status-quo
RELEASE_BASE=${SQ_RELEASE_BASE:-https://github.com/$REPO/releases/latest/download}
RAW_BASE=${SQ_RAW_BASE:-https://raw.githubusercontent.com/$REPO/main/tools}
ASSET=status-quo-linux-x86_64.tar.gz

say() { printf '\n==> %s\n' "$*"; }
ok() { printf '  [ok]  %s\n' "$*"; }
warn() { printf '  [!]   %s\n' "$*"; }
die() { printf '  [x]   %s\n' "$*" >&2; exit 1; }

[ "$(uname -s)" = Linux ] || die "This command is for Linux. On Windows use the PowerShell line in the README."
if command -v curl >/dev/null 2>&1; then
  fetch() { curl -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -q "$1" -O "$2"; }
else
  die "curl or wget is needed to download Status Quo. Install one (for example: sudo apt-get install -y curl) and paste the command again."
fi

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

source_installer() { # source_installer REASON ARGS...
  local reason=$1
  shift
  warn "no prebuilt package for this machine ($reason): using the source installer"
  fetch "$RAW_BASE/install.sh" "$WORK/install.sh" || die "could not download the source installer from $RAW_BASE"
  exec bash "$WORK/install.sh" "$@"
}

say "Status Quo: download"
if [ "$(uname -m)" != x86_64 ]; then
  source_installer "architecture $(uname -m)" "$@"
fi
if ! fetch "$RELEASE_BASE/$ASSET" "$WORK/$ASSET" 2>/dev/null \
  || ! fetch "$RELEASE_BASE/$ASSET.sha256" "$WORK/$ASSET.sha256" 2>/dev/null; then
  source_installer "no release published yet" "$@"
fi
ok "downloaded $ASSET"

if ! (cd "$WORK" && sha256sum -c "$ASSET.sha256" >/dev/null 2>&1); then
  die "checksum does not match: the download is damaged or was changed. Nothing was installed."
fi
ok "checksum ok"

tar -xzf "$WORK/$ASSET" -C "$WORK"
[ -f "$WORK/status-quo-linux-x86_64/install.sh" ] || die "the package has no install.sh inside"
bash "$WORK/status-quo-linux-x86_64/install.sh" "$@"
