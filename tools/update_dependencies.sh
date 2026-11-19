#!/usr/bin/env bash
# Moves Status Quo's dependencies to their newest versions (SQ-53). Run by the scheduled
# workflow .github/workflows/update-dependencies.yml; can be run by hand in a clean clone.
#
#   tools/update_dependencies.sh [OUT_DIR]
#
# What it updates (none of it lives in a workflow file, which the workflow's own key may not
# change; the pinned GitHub actions are kept current by Dependabot, .github/dependabot.yml):
#   - rust-toolchain.toml: the newest stable Rust release;
#   - Cargo.lock: the newest library versions the version ranges in Cargo.toml allow and that
#     still build with `rust-version` (.cargo/config.toml: incompatible-rust-versions);
#   - pipeline-check.version: the newest release of the shared quality check;
#   - tools/build-images: the newest manylinux2014 build images, by digest.
# When the shipped program can change (Rust, libraries, build images) it also raises the patch
# version, so the workflow publishes a release. Writes to OUT_DIR (default build/deps):
#   summary.md   what changed, for the commit message and the run summary
#   release      "yes" or "no"
#   held-back.txt library versions that need a change of Cargo.toml (and likely of code)
set -euo pipefail
cd "$(dirname "$0")/.."
OUT=${1:-build/deps}
mkdir -p "$OUT"
: >"$OUT/summary.md"
note() { printf -- '- %s\n' "$*" | tee -a "$OUT/summary.md"; }
newer() { [ "$1" != "$2" ] && [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | tail -n1)" = "$2" ]; }

# Rust
current=$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)
latest=$(curl -fsSL --retry 3 https://static.rust-lang.org/dist/channel-rust-stable.toml |
  sed -n '/^\[pkg\.rust\]/,/^\[/ s/^version = "\([0-9][0-9.]*\) .*/\1/p' | head -n1)
[ -n "$latest" ] || { echo "the newest Rust release could not be read" >&2; exit 1; }
if newer "$current" "$latest"; then
  sed -i "s/^channel = \".*\"/channel = \"$latest\"/" rust-toolchain.toml
  note "Rust $current -> $latest"
fi
rustup toolchain install >/dev/null

# Libraries
cargo update --verbose 2>&1 | tee "$OUT/cargo-update.log" >/dev/null
grep -E '^\s+(Updating|Adding|Removing|Downgrading) ' "$OUT/cargo-update.log" |
  sed 's/^ *//' | while read -r line; do note "$line"; done
grep -E '^\s+Unchanged .*available' "$OUT/cargo-update.log" | sed 's/^ *//' >"$OUT/held-back.txt" || true

# Shared quality check
current=$(tr -d '[:space:]' <pipeline-check.version)
latest=$(gh release view -R yujideluca/pipeline-check --json tagName -q .tagName)
if [ -n "$latest" ] && newer "${current#v}" "${latest#v}"; then
  printf '%s\n' "$latest" >pipeline-check.version
  note "pipeline-check $current -> $latest"
fi

# Build images
while read -r arch image; do
  case "$arch" in '' | '#'*) continue ;; esac
  name=${image%@*}
  digest=$(docker buildx imagetools inspect "$name:latest" | awk '/^Digest:/ { print $2; exit }')
  [ -n "$digest" ] || { echo "no digest for $name:latest" >&2; exit 1; }
  if [ "$name@$digest" != "$image" ]; then
    sed -i "s|^$arch .*|$arch $name@$digest|" tools/build-images
    note "build image $arch: $digest"
  fi
done < <(cat tools/build-images)

# A release when the shipped program can change.
if git diff --quiet -- rust-toolchain.toml Cargo.lock tools/build-images; then
  echo no >"$OUT/release"
else
  old=$(sed -n '/^\[workspace.package\]/,/^\[/ s/^version = "\(.*\)"/\1/p' Cargo.toml)
  new=${old%.*}.$((${old##*.} + 1))
  sed -i "/^\[workspace.package\]/,/^\[/ s/^version = \"$old\"/version = \"$new\"/" Cargo.toml
  cargo update --workspace >/dev/null 2>&1
  note "Status Quo $old -> $new"
  echo yes >"$OUT/release"
fi
[ -s "$OUT/summary.md" ] || echo "nothing new" >"$OUT/summary.md"
cat "$OUT/summary.md"
