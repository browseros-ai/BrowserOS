#!/bin/sh
set -eu

# Keep the committed input byte-identical to Chromium; build.rs owns surface selection.
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
source_json=${1:-${CDP_PROTOCOL_JSON:-}}
if [ -z "$source_json" ]; then
  echo "usage: CDP_PROTOCOL_JSON=/path/to/protocol.json $0 [protocol.json]" >&2
  exit 2
fi

tmp_dir=$(mktemp -d)
trap 'rm -rf "$tmp_dir"' EXIT HUP INT TERM
cp "$source_json" "$tmp_dir/protocol.json"
jq -e '.domains | type == "array" and length > 0' "$tmp_dir/protocol.json" >/dev/null
sha=$(shasum -a 256 "$tmp_dir/protocol.json" | awk '{print $1}')

cp "$tmp_dir/protocol.json" "$script_dir/protocol/protocol.json"
printf '%s\n' "$sha" >"$script_dir/protocol/protocol.sha256"
printf 'synced %s domains from %s (%s)\n' \
  "$(jq '.domains | length' "$tmp_dir/protocol.json")" "$source_json" "$sha"
