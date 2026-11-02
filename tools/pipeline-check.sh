#!/usr/bin/env bash
# Runs the pipeline-check program at the version pinned in pipeline-check.version (a git tag of
# https://github.com/yujideluca/pipeline-check). The program is built once per version into ~/.cache/pipeline-check.
#   tools/pipeline-check.sh                  full run: gate, architecture, fitness, DORA, GitHub (mode local; ci when CI=true)
#   tools/pipeline-check.sh --measure-only   only architecture and fitness (what the gate itself calls)
# PIPELINE_CHECK_DIR=~/dev/pipeline-check uses a working copy instead of the pinned tag (for developing the program).
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=/dev/null
[ -f ~/.cargo/env ] && source ~/.cargo/env
if [ -n "${PIPELINE_CHECK_DIR:-}" ]; then
  cargo build --release --quiet --manifest-path "$PIPELINE_CHECK_DIR/Cargo.toml"
  exec "$PIPELINE_CHECK_DIR/target/release/pipeline-check" "$@"
fi
ver=$(tr -d '[:space:]' < pipeline-check.version)
cache="${PIPELINE_CHECK_CACHE:-$HOME/.cache/pipeline-check}/$ver"
if [ ! -x "$cache/target/release/pipeline-check" ]; then
  rm -rf "$cache"; mkdir -p "$(dirname "$cache")"
  git clone -q --depth 1 --branch "$ver" "${PIPELINE_CHECK_REPO:-https://github.com/yujideluca/pipeline-check}" "$cache"
  cargo build --release --quiet --manifest-path "$cache/Cargo.toml"
fi
exec "$cache/target/release/pipeline-check" "$@"
