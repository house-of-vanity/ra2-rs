#!/usr/bin/env bash
set -euo pipefail

artifacts=$PWD/artifacts
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
cd "$stage"
files=()
for platform in linux-x86_64 macos-arm64 windows-x86_64; do
  file="ra2-rs-$platform"
  if [[ "$platform" == windows-x86_64 ]]; then file+=.exe; fi
  test -s "$artifacts/$platform/$file"
  cp "$artifacts/$platform/$file" "$file"
  files+=("$file")
  if [[ "$platform" != windows-x86_64 ]]; then
    install -m755 "$file" ra2-rs
    tar --owner=0 --group=0 --numeric-owner --mtime=@0 -cf - ra2-rs | gzip -n > "$file.tar.gz"
    files+=("$file.tar.gz")
  fi
done
sha256sum "${files[@]}" > SHA256SUMS
files+=(SHA256SUMS)

export GH_REPO=${GITHUB_REPOSITORY:?}
tag=${RELEASE_TAG:?}
download="$GITHUB_SERVER_URL/$GH_REPO/releases/download/$tag"
cat > notes.md <<NOTES
| Platform | Download |
| --- | --- |
| Windows x86-64 | [Executable]($download/ra2-rs-windows-x86_64.exe) |
| macOS Apple Silicon | [Archive]($download/ra2-rs-macos-arm64.tar.gz) |
| Linux x86-64 | [Archive]($download/ra2-rs-linux-x86_64.tar.gz) |

Extract the archive on macOS or Linux. Place the executable beside RA2.cfz and Components.cfz in your Robot Arena 2 installation and launch it. The standalone binaries and SHA256SUMS are also available below.
NOTES

if ! draft=$(gh release view "$tag" --json isDraft --jq '.isDraft'); then
  flags=(--draft --verify-tag)
  if [[ "$tag" == *-* ]]; then flags+=(--prerelease); fi
  gh release create "$tag" "${flags[@]}" --title "ra2-rs $tag" --notes-file notes.md
  draft=true
fi

if [[ "$draft" == true ]]; then
  gh release upload "$tag" "${files[@]}" --clobber
else
  # On a rerun, validate the bytes that are already public.
  gh release download "$tag" --pattern SHA256SUMS --dir published
  cp published/SHA256SUMS SHA256SUMS
fi

gh release view "$tag" --json assets > uploaded.json
for file in "${files[@]}"; do
  if [[ "$file" == SHA256SUMS ]]; then
    digest=$(sha256sum SHA256SUMS | cut -d' ' -f1)
  else
    digest=$(awk -v name="$file" '$2 == name {print $1}' SHA256SUMS)
  fi
  [[ "$digest" =~ ^[0-9a-f]{64}$ ]]
  jq -e --arg name "$file" --arg digest "sha256:$digest" \
    'any(.assets[]; .name == $name and .state == "uploaded" and .digest == $digest)' uploaded.json >/dev/null
done

if [[ "$draft" == true ]]; then gh release edit "$tag" --draft=false; fi
gh release view "$tag" --json url --jq '.url'
