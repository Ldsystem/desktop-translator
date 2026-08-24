#!/bin/sh
set -eu

app=${1:?usage: audit-macos-bundle.sh /path/to/Desktop\ Translator.app}
executable="$app/Contents/MacOS/desktop-translator"
test -x "$executable"
test -f "$app/Contents/Info.plist"
codesign --verify --verbose=2 "$app"

bytes=$(stat -f %z "$executable")
limit=$((24 * 1024 * 1024))
if [ "$bytes" -gt "$limit" ]; then
  echo "release executable exceeds compactness budget: $bytes > $limit" >&2
  exit 1
fi

if otool -L "$executable" | grep -E '/opt/homebrew|/usr/local|\.cargo|node|python'; then
  echo "release links a developer-machine runtime" >&2
  exit 1
fi

sidecars=$(find "$app/Contents/MacOS" -type f ! -name desktop-translator | wc -l | tr -d ' ')
if [ "$sidecars" -ne 0 ]; then
  echo "release contains an undeclared sidecar" >&2
  exit 1
fi

external_textbooks=$(find "$app/Contents" -type f -name 'starter-en-zh.sqlite3' | wc -l | tr -d ' ')
if [ "$external_textbooks" -ne 0 ]; then
  echo "bundled textbook must be embedded once in the executable" >&2
  exit 1
fi

app_kib=$(du -sk "$app" | awk '{print $1}')
echo "bundle audit passed: executable=$bytes bytes app=${app_kib}KiB sidecars=$sidecars external_textbooks=$external_textbooks"
