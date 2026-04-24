#!/bin/bash
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$DIR/../.." && pwd)"

cd "$ROOT"

echo "[setup] Installing dependencies..."
bun install --frozen-lockfile

echo "[setup] Generating agent code..."
bun run codegen:agent

echo "[setup] Syncing VM cache..."
printf '\033[31m[setup] First VM cache sync can take about 5 minutes.\033[0m\n'
NODE_ENV=development bun run --filter @browseros/build-tools cache:sync

echo "[setup] Ready"
