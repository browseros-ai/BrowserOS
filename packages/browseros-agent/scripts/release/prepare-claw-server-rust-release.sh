#!/usr/bin/env bash
set -euo pipefail

# Resolve a BrowserClaw server GitHub Release from the Rust crate version while
# preserving the established claw-server-rust tag identity.
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"

exec "$script_dir/prepare-server-bundle-release.sh" \
  --release-name "BrowserClaw Server (Rust)" \
  --component-name "claw server rust" \
  --tag-prefix "claw-server-rust/v" \
  --cargo-toml "packages/browseros-agent/apps/claw-server-rust/Cargo.toml" \
  "$@"
