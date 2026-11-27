#!/usr/bin/env bash
set -euo pipefail

platform=${1:?specify the release platform}
toolchain=$(sed -n 's/^rust-version = "\([^"]*\)"$/\1/p' Cargo.toml)
version=$(sed -n 's/^version = "\([^"]*\)"$/\1/p' Cargo.toml | head -n 1)
if [[ -z "$toolchain" || "${RELEASE_TAG:?}" != "v$version" ]]; then
  echo "Expected a Rust toolchain and release tag v$version in Cargo.toml" >&2
  exit 1
fi

extension=
case "$platform" in
  linux-x86_64)
    target=x86_64-unknown-linux-gnu
    apt-get update -q
    apt-get install -y --no-install-recommends build-essential pkg-config curl ca-certificates \
      libasound2-dev libwayland-dev libxkbcommon-dev
    curl --fail --silent --show-error --location --retry 3 https://sh.rustup.rs -o /tmp/ra2-rustup.sh
    sh /tmp/ra2-rustup.sh -y --profile minimal --default-toolchain "$toolchain"
    export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
    ;;
  macos-arm64)
    target=aarch64-apple-darwin
    ;;
  windows-x86_64)
    target=x86_64-pc-windows-msvc
    extension=.exe
    export CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS="-C target-feature=+crt-static"
    ;;
  *) echo "Unknown release platform: $platform" >&2; exit 1 ;;
esac

rustup toolchain install "$toolchain" --profile minimal --target "$target"
cargo "+$toolchain" build --locked --release --bin ra2-rs --target "$target"
binary="target/$target/release/ra2-rs$extension"
actual=$("$binary" --version)
if [[ "${actual%$'\r'}" != "ra2-rs $version" ]]; then
  echo "Unexpected release version: $actual" >&2
  exit 1
fi
mkdir -p dist
cp "$binary" "dist/ra2-rs-$platform$extension"
