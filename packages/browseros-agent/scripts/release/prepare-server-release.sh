#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"

exec "$script_dir/prepare-server-bundle-release.sh" \
  --release-name "BrowserOS Server" \
  --component-name "server" \
  --tag-prefix "agent-server/v" \
  --legacy-prefix "browseros-server-v" \
  --package-json "packages/browseros-agent/apps/server/package.json" \
  "$@"
