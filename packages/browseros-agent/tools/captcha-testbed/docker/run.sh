#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../../.."
exec bun tools/captcha-testbed/docker/run.ts "$@"
