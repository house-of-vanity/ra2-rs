#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
export WGPU_BACKEND="${WGPU_BACKEND:-vulkan}"
export CARGO_HOME="$PWD/.cargo-home"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
export XDG_CACHE_HOME="$PWD/.cache"
if [[ ${RA2_RS_SHELL:-} == 1 ]]; then
    cargo_profile=()
    if [[ ${1:-} == --release ]]; then
        cargo_profile=(--release)
        shift
    fi
    if [[ ! -f Cargo.lock ]]; then
        cargo generate-lockfile
    fi
    exec cargo run --locked "${cargo_profile[@]}" -- "$@"
fi
# Bash-escape every argument before passing a command to nix-shell.
printf -v launch_command '%q ' ./run.sh "$@"
exec nix-shell --run "$launch_command"
