#!/usr/bin/env bash
# launch-browseros-dev.sh — GUI launcher for BrowserOS Dev instance
# Called by browseros-dev.desktop Exec line.
# Ensures singleton guard, port checks, and data preservation.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

# Source shared functions
source "${SCRIPT_DIR}/common.sh"

PID_FILE="${DEV_BOS_DIR}/browser.pid"
LOCK_FILE="${DEV_BOS_DIR}/browser.lock"

# ── Ensure mise is available ──
if ! command -v mise &>/dev/null; then
  notify-send "BrowserOS Dev" "mise not found. Install mise first." -i dialog-error 2>/dev/null || true
  echo "[ERROR] mise not found in PATH" >&2
  exit 1
fi

# ── Check if already running ──
if is_browser_running "$PID_FILE"; then
  notify-send "BrowserOS Dev" "Already running. Use kill-dev to stop first." -i dialog-warning 2>/dev/null || true
  echo "[INFO] BrowserOS Dev already running." >&2
  exit 0
fi

# ── Launch via mise (ensures build + singleton guard) ──
cd "$REPO_ROOT"
exec mise run browseros:start-dev
