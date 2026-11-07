#!/usr/bin/env bash
# One definition of passing, called unchanged by CI and the pinned pipeline runner.
set -euo pipefail
cd "$(dirname "$0")/.."
MAX_GATE_SECONDS=1200
SECONDS=0
step() { printf '\n== %s\n' "$1"; }
step "1 Rust formatting"
cargo fmt --all -- --check
rustfmt --check --edition 2021 tools/rust_architecture.rs tools/resource_check.rs
step "1b test scratch folders delete themselves"
# A test that builds temp_dir()/name-PID by hand leaves it behind when it fails or forgets to remove
# it, and a rerun has a new PID, so the folders pile up in /tmp (RAM on this laptop). Test folders
# come only from crates/scratch.rs (Scratch, removed on drop); any other temp_dir() line must say
# `scratch-ok:` and why.
stray=$(grep -rn --include='*.rs' 'temp_dir()' crates tools | grep -v '^crates/scratch.rs:' | grep -v 'scratch-ok:' || true)
[ -z "$stray" ] || { printf 'hand-built temp folder (use crates/scratch.rs Scratch):\n%s\n' "$stray"; exit 1; }
step "2 compiler diagnostics and tests"
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
step "3 portable Rust boundaries and detector regression checks"
mkdir -p build
rustc --edition 2021 -D warnings tools/rust_architecture.rs -o build/rust-architecture
build/rust-architecture --self-test
build/rust-architecture
rustc --edition 2021 -D warnings tools/resource_check.rs -o build/resource-check
build/resource-check --self-test
step "4 requirement traceability"
tools/pipeline-check.sh --traceability
git diff --quiet -- docs/TRACEABILITY.md || { echo "docs/TRACEABILITY.md changed: commit the regenerated matrix"; exit 1; }
step "5 fitness functions"
tools/pipeline-check.sh --measure-only
[ "$SECONDS" -le "$MAX_GATE_SECONDS" ] || { echo "FAIL gate took ${SECONDS}s (> ${MAX_GATE_SECONDS}s)"; exit 1; }
if [ "${STATUS_QUO_E2E:-0}" = 1 ]; then
  step "6 desktop end-to-end test (needs a Wayland session and a free screen: also set STATUS_QUO_E2E_SCREEN_IS_FREE=1)"
  tools/ui_e2e.sh
fi
printf '\nquality gate passed (%ss)\n' "$SECONDS"
