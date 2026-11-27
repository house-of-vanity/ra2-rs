#!/usr/bin/env bash
set -euo pipefail

source /etc/os-release
case "$ID" in
  ubuntu|debian)
    apt-get update -q
    apt-get install -y --no-install-recommends libasound2 libwayland-client0 libwayland-cursor0 \
      libxkbcommon0 libx11-6 libxcursor1 libxi6 libxrandr2 libvulkan1
    ;;
  arch)
    pacman -Syu --noconfirm --needed alsa-lib wayland libxkbcommon libx11 libxcursor libxi libxrandr vulkan-icd-loader
    ;;
  *) echo "Unknown release test distribution: $ID" >&2; exit 1 ;;
esac

# Artifact downloads do not preserve executable permissions. Run from outside
# the checkout, with only the exact file that will be attached to the release.
install -Dm755 dist/ra2-rs-linux-x86_64 '/tmp/Robot Arena 2/ra2-rs'
cd /tmp
actual=$('/tmp/Robot Arena 2/ra2-rs' --version)
echo "$PRETTY_NAME: $actual"
[[ "$actual" == "ra2-rs ${RELEASE_TAG#v}" ]]
