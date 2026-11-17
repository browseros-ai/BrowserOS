#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../../../../.."
exec bun crates/neo-sdk/neo-captcha/testbed/docker/run.ts "$@"
