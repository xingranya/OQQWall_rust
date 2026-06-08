#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="$ROOT_DIR/crates/app/webview-ui/dist"
INDEX_HTML="$DIST_DIR/index.html"

if [[ ! -f "$INDEX_HTML" ]]; then
  echo "error: missing WebView dist index: $INDEX_HTML" >&2
  echo "hint: run bun run build in crates/app/webview-ui and commit the dist output" >&2
  exit 1
fi

missing=0
while IFS= read -r asset; do
  [[ -z "$asset" ]] && continue
  asset="${asset#/}"
  if [[ ! -f "$DIST_DIR/$asset" ]]; then
    echo "error: WebView dist references missing asset: $asset" >&2
    missing=1
  fi
done < <(
  grep -Eo '(src|href)="[^"]+"' "$INDEX_HTML" \
    | sed -E 's/^(src|href)="([^"]+)"$/\2/' \
    | grep -E '^/?assets/' || true
)

if [[ "$missing" -ne 0 ]]; then
  echo "hint: rebuild the WebView frontend and commit dist/index.html with its dist/assets files" >&2
  exit 1
fi

echo "webview dist ok"
