#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"

exec "$script_dir/prepare-server-bundle-release.sh" \
  --release-name "BrowserClaw Server" \
  --component-name "claw server" \
  --tag-prefix "claw-server/v" \
  --cargo-toml "packages/browseros-agent/apps/claw-server-rust/Cargo.toml" \
  "$@"
