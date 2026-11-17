#!/usr/bin/env bash
#
# The Neo SDK image (claw-sdk: one MCP tool, `run`, plus a host API) in Docker, from this checkout,
# with the dev kit (devkit/; devkit/HOWTO.md walks through it). The default Neo image (claw-server,
# production) has its own dev script in packages/browseros/docker/neo.
#
#   ./dev.sh up              Build this checkout's SDK image (build.sh), start it, start the dev
#                            kit's gateway, print URLs. Run it again after every change.
#   ./dev.sh run [CODE | - | -f FILE] [--task ID] [--timeout MS] [--request-id ID] [--json]
#                            Send one script to `run` (`-` reads stdin). Prints what it returned
#                            and `run <id> <status> effects=<e> <ms>ms tabs=[..]`, and appends the
#                            call to $NEO_DEV_HOME/traces/runs.jsonl.
#   ./dev.sh tools [name]    List the MCP tools, or one tool's description and schema.
#   ./dev.sh call <tool> [json-args]
#                            Call one MCP tool and print its result.
#   ./dev.sh host <METHOD> <path> [json-body]
#                            Call the host API, e.g. `host GET '/v1/runs?taskId=demo'`.
#   ./dev.sh export          Copy every run, operation, screenshot and recent Neo event to
#                            $NEO_DEV_HOME/traces/export-<time>/.
#   ./dev.sh gateway         Restart the gateway alone (after editing devkit/).
#   ./dev.sh logs            Follow the container's logs (entrypoint, browser, server).
#   ./dev.sh smoke           smoke_sdk.py from the Mac, then check_display.py in the container.
#   ./dev.sh down            Stop the container and the gateway; the profile is kept.
#   ./dev.sh reset           Stop as `down` does and delete the profile.
#
#   Viewer: http://127.0.0.1:16090    Trace UI: http://127.0.0.1:16090/trace
#   MCP: http://127.0.0.1:19010/mcp   Host API: http://127.0.0.1:19300/v1, bearer token in
#   $NEO_DEV_HOME/host-token
#   devkit/examples/check.sh runs the example scripts against the container.
#
# Setup, once: Docker, jq, openssl, npx (the gateway runs on Bun 1.4.2), and `bun install` in
# packages/browseros-agent (the viewer page loads noVNC from the neo-viewer tool's package).
#
# Testing a change (for agents): `./dev.sh up`, give the owner the viewer URL, and drive Neo only
# through `./dev.sh run` (the MCP surface is what ships; Playwright, raw CDP or a headless browser
# prove nothing about it). `--task` keeps a run on its task's last tab. Check `./dev.sh logs` and
# run `./dev.sh smoke` before handing off.
#
# Settings (environment):
#   NEO_MCP_PORT, NEO_HOST_PORT  Host ports for MCP (19010) and the host API (19300).
#   NEO_GATEWAY_PORT             The dev kit's viewer and trace UI (16090).
#   NEO_DEV_HOME                 Profile, host token and traces (~/.local/state/neo-sdk-dev),
#                                outside the repo so worktrees share one signed-in profile.
#   NEO_SDK_BUILD_JOBS           Caps Cargo's parallel jobs in the image build (build.sh).
#
# One SDK dev Neo per machine: `up` from another worktree replaces the running container with that
# worktree's image. The compose project is `neo-sdk-dev`.

set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
AGENT_DIR=$(cd "$HERE/../../../.." && pwd)
DEV_IMAGE=neo-sdk-dev:local
export NEO_MCP_PORT=${NEO_MCP_PORT:-19010}
export NEO_HOST_PORT=${NEO_HOST_PORT:-19300}
NEO_GATEWAY_PORT=${NEO_GATEWAY_PORT:-16090}
# Exported for compose.dev.yaml, which mounts the data folder and the host token from here.
export NEO_DEV_HOME=${NEO_DEV_HOME:-$HOME/.local/state/neo-sdk-dev}
export NEO_DATA=$NEO_DEV_HOME/data
export NEO_IMAGE=$DEV_IMAGE
HOST_TOKEN=$NEO_DEV_HOME/host-token
HOST_URL=http://127.0.0.1:$NEO_HOST_PORT
TRACES=$NEO_DEV_HOME/traces
GATEWAY_PID=$NEO_DEV_HOME/gateway.pid
GATEWAY_LOG=$NEO_DEV_HOME/gateway.log
BUN_VERSION=1.4.2
# The npm package of noVNC the neo-viewer tool depends on: its RFB client speaks the host API's
# /v1/display. The SDK image itself carries no noVNC.
NOVNC=$AGENT_DIR/tools/neo-viewer/node_modules/@novnc/novnc

compose() { docker compose -p neo-sdk-dev -f "$HERE/compose.dev.yaml" "$@"; }
die() { echo "dev.sh: $*" >&2; exit 1; }
need() { command -v "$1" >/dev/null || die "needs $1; see Setup in this script's header"; }

up() {
  need jq; need openssl; need npx
  [ -f "$NOVNC/core/rfb.js" ] || die "no noVNC at $NOVNC; run \`bun install\` in $AGENT_DIR"
  bash "$HERE/build.sh" --tag "$DEV_IMAGE"
  mkdir -p "$NEO_DATA"
  ensure_host_token
  compose up -d --wait
  local running
  running=$(compose exec -T neo neo-server-info kind)
  [ "$running" = claw-sdk ] || die "built claw-sdk, but the container runs $running"
  start_gateway
  local mcp_url=http://127.0.0.1:$NEO_MCP_PORT/mcp
  echo
  gateway_urls
  echo "MCP:      $mcp_url"
  echo "Host API: $HOST_URL/v1   bearer token in $HOST_TOKEN"
  echo
  echo "Claude Code:"
  echo "  claude mcp add --transport http neo-sdk $mcp_url"
}

# The host API's bearer token. Whoever holds it can read every trace and take over the browser,
# so it is made once, owner-only, and never printed: every command reads it from this file.
# Written to a temp file and renamed so an interrupted run never leaves a short token that
# claw-sdk would refuse.
ensure_host_token() {
  [ -s "$HOST_TOKEN" ] && return
  mkdir -p "$NEO_DEV_HOME"
  (umask 077 && openssl rand -hex 32 >"$HOST_TOKEN.tmp" && mv "$HOST_TOKEN.tmp" "$HOST_TOKEN")
}

# The gateway (devkit/gateway.ts) is the browser's door to the host API: it adds the bearer token,
# so no page ever holds it, and serves the viewer and the trace UI. It runs as Bun's own binary,
# resolved through npx once, so the pid stored here is the server itself. nohup and the redirects
# detach it from this shell, so it outlives `dev.sh` and the terminal.
start_gateway() {
  stop_gateway
  local bun pid
  bun=$(npx -y "bun@$BUN_VERSION" -e 'console.log(process.execPath)') || die "can't fetch Bun $BUN_VERSION with npx"
  mkdir -p "$TRACES"
  nohup "$bun" "$HERE/devkit/gateway.ts" --port "$NEO_GATEWAY_PORT" --upstream "$HOST_URL" \
    --token-file "$HOST_TOKEN" --novnc-dir "$NOVNC" --traces-dir "$TRACES" \
    </dev/null >"$GATEWAY_LOG" 2>&1 &
  pid=$!
  echo "$pid" >"$GATEWAY_PID"
  # Ready means this process printed its listening line and the port answers, so an older
  # gateway still holding the port can't pass for the new one.
  for _ in $(seq 50); do
    kill -0 "$pid" 2>/dev/null || break
    if grep -q '^gateway: ' "$GATEWAY_LOG" &&
      [ "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$NEO_GATEWAY_PORT/dev/runs")" != 000 ]; then
      return
    fi
    sleep 0.2
  done
  echo "dev.sh: the gateway did not come up on port $NEO_GATEWAY_PORT; the end of $GATEWAY_LOG:" >&2
  tail -n 20 "$GATEWAY_LOG" >&2
  stop_gateway
  exit 1
}

# A pid file can outlive its process and the pid be reused, so only a process that is still
# running gateway.ts gets signalled.
stop_gateway() {
  [ -f "$GATEWAY_PID" ] || return 0
  local pid
  pid=$(cat "$GATEWAY_PID")
  if ps -p "$pid" -o command= 2>/dev/null | grep -q 'devkit/gateway\.ts'; then
    kill "$pid" 2>/dev/null || true
    for _ in $(seq 25); do
      kill -0 "$pid" 2>/dev/null || break
      sleep 0.2
    done
    kill -9 "$pid" 2>/dev/null || true
  fi
  rm -f "$GATEWAY_PID"
}

gateway_urls() {
  echo "Viewer:   http://127.0.0.1:$NEO_GATEWAY_PORT/   (?mode=control to type into it)"
  echo "Trace UI: http://127.0.0.1:$NEO_GATEWAY_PORT/trace"
}

need_token() { [ -s "$HOST_TOKEN" ] || die "no host token at $HOST_TOKEN; run \`dev.sh up\`"; }

# The host API from the shell. The token reaches curl as config on stdin, not as an argument, so
# it never shows in `ps` or in shell history.
host() {
  [ $# -ge 2 ] || die "usage: host <METHOD> <path> [json-body]"
  need_token
  local method=$1 path=$2 out status kind
  local body=()
  [ $# -ge 3 ] && body=(-H 'Content-Type: application/json' --data-binary "$3")
  out=$(mktemp)
  read -r status kind < <(printf 'header = "Authorization: Bearer %s"\n' "$(tr -d '[:space:]' <"$HOST_TOKEN")" |
    curl -sS -K - -X "$method" -o "$out" -w '%{http_code} %{content_type}\n' \
      ${body[@]+"${body[@]}"} "$HOST_URL/${path#/}") || true
  if [[ $kind == image/* ]]; then
    mkdir -p "$NEO_DEV_HOME/shots"
    local shot
    shot="$NEO_DEV_HOME/shots/$(date +%Y%m%d-%H%M%S)-host-$RANDOM.${kind#image/}"
    mv "$out" "$shot"
    echo "[image saved: $shot]"
  elif [[ $status == 2?? ]]; then
    jq . "$out" 2>/dev/null || cat "$out"
  else
    echo "HTTP ${status:-000} $method /${path#/}" >&2
    jq . "$out" >&2 2>/dev/null || cat "$out" >&2
  fi
  rm -f "$out"
  [[ $status == 2?? ]]
}

mcp() { python3 "$HERE/dev_mcp.py" --url "http://127.0.0.1:$NEO_MCP_PORT/mcp" --state "$NEO_DEV_HOME" "$@"; }

run() {
  need_token
  mcp run --traces-dir "$TRACES" --host-url "$HOST_URL" --token-file "$HOST_TOKEN" "$@"
}

smoke() {
  python3 "$HERE/smoke_sdk.py" --mcp-port "$NEO_MCP_PORT"
  compose exec -T neo python3 - claw-sdk <"$HERE/check_display.py"
}

case ${1:-} in
  up) shift; up "$@" ;;
  tools|call) mcp "$@" ;;
  run) shift; run "$@" ;;
  host) shift; host "$@" ;;
  export) need_token; python3 "$HERE/devkit/export.py" --host-url "$HOST_URL" --token-file "$HOST_TOKEN" --traces-dir "$TRACES" ;;
  gateway) need_token; start_gateway; gateway_urls ;;
  logs) compose logs -f ;;
  smoke) smoke ;;
  down) stop_gateway; compose down ;;
  reset) stop_gateway; compose down; rm -rf "$NEO_DATA" ;;
  *) sed -n '3,/^set -euo/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
