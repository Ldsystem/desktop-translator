#!/bin/sh
set -eu
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
output="$script_dir/../../src-tauri/resources/textbooks/starter-en-zh.sqlite3"
mkdir -p "$(dirname -- "$output")"
rm -f "$output"
sqlite3 "$output" < "$script_dir/starter-en-zh.sql"
shasum -a 256 "$output"
wc -c "$output"
