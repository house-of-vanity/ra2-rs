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

api="${GITHUB_API_URL%/}/repos/$GITHUB_REPOSITORY/releases"
download="${GITHUB_SERVER_URL%/}/$GITHUB_REPOSITORY/releases/download/$RELEASE_TAG"
auth="Authorization: token ${RELEASE_TOKEN:?}"
body="Скачайте [Windows x86-64]($download/ra2-rs-windows-x86_64.exe), [Linux x86-64]($download/ra2-rs-linux-x86_64.tar.gz) или [macOS ARM64]($download/ra2-rs-macos-arm64.tar.gz). На Linux и macOS извлеките файл из архива, чтобы сохранить право на запуск."
body+=$'\n\n'
body+='Положите исполняемый файл рядом с Components.cfz и RA2.cfz установленной Robot Arena 2 и запустите его. Для сборки установщиков доступны отдельные бинарники и SHA256SUMS.'

request() { curl --silent --show-error --fail-with-body -H "$auth" "$@"; }
status=$(curl --silent --show-error -H "$auth" -o release.json -w '%{http_code}' "$api/tags/$RELEASE_TAG")
case "$status" in
  404)
    jq -n --arg tag "$RELEASE_TAG" --arg body "$body" \
      '{tag_name: $tag, name: ("ra2-rs " + $tag), body: $body, draft: true, prerelease: ($tag | contains("-"))}' > create.json
    request -H 'Content-Type: application/json' --data-binary @create.json "$api" > release.json
    ;;
  200) ;;
  *) echo "Release lookup returned HTTP $status" >&2; exit 1 ;;
esac
id=$(jq -er '.id' release.json)
if [[ $(jq -r '.draft' release.json) == false ]]; then
  # A repeated run must never mix two different builds under a public tag.
  request -L "$download/SHA256SUMS" | cmp - SHA256SUMS
  for file in "${files[@]}"; do
    jq -e --arg file "$file" 'any(.assets[]; .name == $file)' release.json >/dev/null
  done
  exit 0
fi

for file in "${files[@]}"; do
  # A previous attempt may have stopped partway through this draft.
  previous=$(jq -r --arg file "$file" '.assets[]? | select(.name == $file) | .id' release.json)
  if [[ -n "$previous" ]]; then request -X DELETE "$api/$id/assets/$previous" >/dev/null; fi
  request -F "attachment=@$file" "$api/$id/assets?name=$file" > uploaded.json
  jq -e --arg name "$file" --argjson size "$(stat -c %s "$file")" \
    '.name == $name and .size == $size' uploaded.json >/dev/null
done
request -X PATCH -H 'Content-Type: application/json' -d '{"draft":false}' "$api/$id" \
  | jq -er 'select(.draft == false) | .html_url'
