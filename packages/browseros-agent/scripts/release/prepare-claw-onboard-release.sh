#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"

exec "$script_dir/prepare-server-bundle-release.sh" \
  --release-name "BrowserClaw Onboarding" \
  --component-name "claw onboarding" \
  --tag-prefix "claw-onboard/v" \
  --package-json "packages/browseros-agent/apps/claw-onboard/package.json" \
  "$@"
