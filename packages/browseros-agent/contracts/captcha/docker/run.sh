#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../../.."
exec bun contracts/captcha/docker/run.ts "$@"
