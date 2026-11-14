#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
export WGPU_BACKEND="${WGPU_BACKEND:-vulkan}"
export CARGO_HOME="$PWD/.cargo-home"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
export XDG_CACHE_HOME="$PWD/.cache"
if [[ ${PHYSICS_DEMO_SHELL:-} == 1 ]]; then
    demo_profile=()
    if [[ ${1:-} == --release ]]; then
        demo_profile=(--release)
        shift
    fi
    if [[ ! -f Cargo.lock ]]; then
        cargo generate-lockfile
    fi
    # Preserve this checkout's developer preferences. The standalone binary
    # uses its embedded configuration and the selected game's .open-arena/.
    exec cargo run --locked "${demo_profile[@]}" -- \
        --config "$PWD/config/demo.ron" \
        --settings-file "$PWD/config/user-settings.ron" "$@"
fi
# Bash-escape every argument before passing a command to nix-shell.
printf -v demo_command '%q ' ./run.sh "$@"
exec nix-shell --run "$demo_command"
