#!/usr/bin/env bash
set -Eeuo pipefail

# Python owns the whole prepare-and-copy transaction. In particular, a shell
# `source ensure; cp` sequence would release the base lock before copying it.
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd "$script_dir/../.." && pwd -P)"
package_root="$repo_root/packages/browseros"
run_tag="${GITHUB_RUN_ID:-local}-${GITHUB_RUN_ATTEMPT:-1}"

if [ -z "${RUNNER_TEMP:-}" ]; then
  echo "::error::RUNNER_TEMP is required for owned workspace recovery" >&2
  exit 1
fi

case "${1:-}" in
  setup)
    base_src="${2:-${CHROMIUM_SRC:-}}"
    products="${3:-browseros}"
    if [ -z "$base_src" ]; then
      echo "usage: $0 setup <persistent-chromium-src> [browseros|browserclaw|all]" >&2
      exit 2
    fi
    exec uv run --project "$package_root" browseros source workspace \
      --base-src "${base_src/#\~/$HOME}" \
      --products "$products" \
      --version-file "${BROWSEROS_CHROMIUM_VERSION_FILE:-$package_root/CHROMIUM_VERSION}" \
      --runner-temp "$RUNNER_TEMP" \
      --run-tag "$run_tag"
    ;;
  cleanup)
    # Discover both product-qualified ledgers even if setup failed before it
    # could publish any step outputs, or the second product only partly copied.
    exec uv run --project "$package_root" browseros source cleanup-workspaces \
      --runner-temp "$RUNNER_TEMP" \
      --run-tag "$run_tag" \
      --products "${2:-all}"
    ;;
  *)
    echo "usage: $0 setup <persistent-chromium-src> [browseros|browserclaw|all]|cleanup" >&2
    exit 2
    ;;
esac
