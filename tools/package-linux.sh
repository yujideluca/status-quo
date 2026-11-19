#!/bin/sh
# Builds the Linux download, status-quo-linux-<processor>.tar.gz (SQ-45, SQ-47).
#
#   sh tools/package-linux.sh
#
# The release runs this inside an old base system (the manylinux2014 image, glibc 2.17), so the
# program starts on old distributions as well as new ones: a program built on a new system
# refuses to start on an older one (v0.1.0 to v0.1.2 needed glibc 2.35). The build needs no
# system development packages. Rust is installed into build/manylinux when it is missing.
# The result is checked with tools/check_binary.rs and written to build/pkg/.
set -eu
cd "$(dirname "$0")/.."
ROOT=$PWD
ARCH=$(uname -m)
NAME=status-quo-linux-$ARCH
# The Rust release of rust-toolchain.toml (kept current by tools/update_dependencies.sh).
RUST=$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)

if ! command -v cargo >/dev/null 2>&1 || ! cargo --version | grep -q " ${RUST%.*}"; then
  export RUSTUP_HOME="$ROOT/build/manylinux/rustup" CARGO_HOME="$ROOT/build/manylinux/cargo"
  export PATH="$CARGO_HOME/bin:$PATH"
  if ! command -v cargo >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain "$RUST" --no-modify-path
  fi
  export CARGO_TARGET_DIR="$ROOT/build/manylinux/target"
fi
TARGET=${CARGO_TARGET_DIR:-$ROOT/target}

cargo build --locked --release -p status-quo
mkdir -p build/pkg
rustc --edition 2021 -D warnings tools/check_binary.rs -o build/pkg/check-binary
build/pkg/check-binary --max-glibc 2.17 "$TARGET/release/status-quo"

rm -rf "build/pkg/$NAME"
mkdir -p "build/pkg/$NAME"
cp "$TARGET/release/status-quo" "build/pkg/$NAME/status-quo"
cp tools/install-prebuilt.sh "build/pkg/$NAME/install.sh"
cp LICENSE README.md "build/pkg/$NAME/"
chmod +x "build/pkg/$NAME/install.sh" "build/pkg/$NAME/status-quo"
tar -C build/pkg -czf "build/pkg/$NAME.tar.gz" "$NAME"
(cd build/pkg && sha256sum "$NAME.tar.gz" >"$NAME.tar.gz.sha256")
rm -rf "build/pkg/$NAME" build/pkg/check-binary
echo "packed build/pkg/$NAME.tar.gz"
