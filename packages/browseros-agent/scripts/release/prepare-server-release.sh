#!/usr/bin/env bash
set -euo pipefail

# Resolve a server GitHub Release. The tag is the source of truth for the version;
# this script never pushes to the default branch. On manual dispatch it creates and
# pushes only the annotated tag (allowed under a "changes to main via PR" ruleset);
# the version is reflected back into package.json by the workflow's bump PR step.
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"

exec "$script_dir/prepare-server-bundle-release.sh" \
  --release-name "BrowserOS Server" \
  --component-name "server" \
  --tag-prefix "agent-server/v" \
  --legacy-prefix "browseros-server-v" \
  --package-json "packages/browseros-agent/apps/server/package.json" \
  "$@"
