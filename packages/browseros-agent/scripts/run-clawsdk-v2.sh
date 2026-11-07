#!/usr/bin/env bash
# Starts the BrowserOS neo dev watch on the claw SDK v2 server (the Playwright run prototype).
# MCP then offers one `run` tool at http://127.0.0.1:9200/mcp; it is ready once the log shows
# "Playwright prototype listening". Extra arguments go to the watcher. BROWSERCLAW_DIR overrides
# the Claw state directory. Stop with Ctrl-C or `bun run dev:stop`.
set -euo pipefail
cd "$(dirname "$0")/.."

export BROWSERCLAW_DIR="${BROWSERCLAW_DIR:-$HOME/.browserclaw-copy-prod}"
export BROWSERCLAW_PLAYWRIGHT_RUN=1
# Same as `bun run dev:claw:watch`; the watcher hands this environment to claw-server.
exec ./tools/dev/run.sh watch --claw "$@"
