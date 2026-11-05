#!/usr/bin/env bash
#
# Local BrowserOS neo in Docker, running this checkout's Neo server, for testing
# server changes the way hosted users (zuzu, other agent backends) see them.
#
#   ./dev.sh up [--server claw-server|claw-sdk]
#                            Build that server (default claw-server) from this
#                            checkout, bake it into the image (this folder's
#                            Dockerfile), start Neo, print URLs. Run it again after
#                            every server change.
#   ./dev.sh up --published [tag]
#                            Run ghcr.io/browseros-ai/neo:<tag> (default latest)
#                            unchanged, to compare against what is released.
#   ./dev.sh tools [name]    List Neo's MCP tools, or one tool's description and schema.
#   ./dev.sh call <tool> [json-args]
#                            Call one MCP tool and print its result. Images are saved
#                            to files and their paths printed.
#   ./dev.sh logs            Follow the container's logs (entrypoint, browser, server).
#   ./dev.sh smoke           Smoke-test the server the container runs: smoke.py, or for
#                            claw-sdk smoke_sdk.py plus check_display.py.
#   ./dev.sh down            Stop the container and the dev kit's gateway; the profile
#                            and logins are kept.
#   ./dev.sh reset           Stop as `down` does and delete the profile.
#
#   Viewer (noVNC): http://localhost:16080      MCP: http://localhost:19010/mcp
#
# claw-sdk (one MCP tool, `run`, plus a host API); devkit/HOWTO.md walks through it.
# `up --server claw-sdk` also starts the dev kit's gateway and prints its URLs:
#   Viewer: http://127.0.0.1:16090    Trace UI: http://127.0.0.1:16090/trace
#   Host API: http://127.0.0.1:19300/v1, bearer token in $NEO_DEV_HOME/host-token
#
#   ./dev.sh run [CODE | - | -f FILE] [--task ID] [--timeout MS] [--request-id ID] [--json]
#                            Send one script to `run` (`-` reads stdin). Prints what it
#                            returned and `run <id> <status> effects=<e> <ms>ms tabs=[..]`,
#                            and appends the call to $NEO_DEV_HOME/traces/runs.jsonl.
#   ./dev.sh host <METHOD> <path> [json-body]
#                            Call the host API, e.g. `host GET '/v1/runs?taskId=demo'`.
#   ./dev.sh export          Copy every run, operation, screenshot and recent Neo event
#                            to $NEO_DEV_HOME/traces/export-<time>/.
#   ./dev.sh gateway         Restart the gateway alone (after editing devkit/).
#   devkit/examples/check.sh runs the example scripts against the container.
#
# Setup, once: Docker, plus what the release workflow's server job installs to
# cross-build the server for Linux:
#   brew install jq zig && cargo install --locked cargo-zigbuild
#   rustup toolchain install 1.98.0
#   rustup +1.98.0 target add aarch64-unknown-linux-gnu    # x86_64-unknown-linux-gnu on Intel
# The claw-sdk kit also needs npx (it runs the gateway on Bun 1.4.2) and openssl.
#
# Testing a change (for agents):
#   1. `./dev.sh up`, then give the owner the viewer URL; they watch the agent there.
#   2. Drive Neo only through its own MCP tools via `./dev.sh call` (claw-server) or
#      `./dev.sh run` (claw-sdk), never Playwright, Puppeteer, raw CDP or a headless
#      browser: the MCP surface is what ships, so a test through anything else proves
#      nothing about it. Read the schemas with `./dev.sh tools <name>` rather than
#      guessing argument names.
#        ./dev.sh call tabs '{"action":"new","url":"https://example.com"}'
#        ./dev.sh call snapshot '{"page":2}'
#        ./dev.sh call screenshot '{"page":2}'     # prints a .png path; read it
#        ./dev.sh run --task demo "await page.goto('https://example.com'); return page.title()"
#      Prefer `run` (one async JS script against the `browser` SDK) for multi-step flows.
#   3. claw-server calls share one MCP session (dev_mcp.py), so pages opened by one call
#      belong to the next. When Neo drops it (30 idle minutes, a new container), the next
#      call starts a fresh one by itself. claw-sdk has no sessions: `--task` keeps a run
#      on its task's last tab.
#   4. Check `./dev.sh logs` for server errors, and run `./dev.sh smoke` before handing
#      off. Leave pages open for the owner to inspect.
#   A Claude Code session can also add Neo as an MCP server, though only sessions
#   started afterwards see its tools:
#      claude mcp add --transport http neo-docker http://localhost:19010/mcp
#
# Settings (environment):
#   NEO_VNC_PORT, NEO_MCP_PORT  Host ports (16080, 19010), clear of a desktop Neo's.
#   NEO_HOST_PORT               claw-sdk's host API on the Mac (19300).
#   NEO_GATEWAY_PORT            The dev kit's viewer and trace UI (16090).
#   NEO_DEV_HOME                Profile, staged server, MCP session, saved images, host
#                               token and traces (~/.local/state/neo-dev). Kept outside
#                               the repo so worktrees share one signed-in profile, and so
#                               neither the profile nor page screenshots ever enter this
#                               folder's Docker build context.
#   NEO_BROWSER_VERSION         Neo browser package `up` builds on (default: the
#                               browser in the published `latest` image).
#   NEO_RUST_TOOLCHAIN          Rust toolchain for the server build (1.98.0, the release
#                               job's); set it empty to use rustup's default.
#
# One dev Neo per machine: `up` from another worktree replaces the running container
# with that worktree's server. The compose project is `neo-dev`; it runs
# compose.yaml unchanged, with the image, ports and data folder set below, and for
# claw-sdk adds compose.sdk-dev.yaml (dev only; its comments say why).

set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
AGENT_DIR=$(cd "$HERE/../../../browseros-agent" && pwd)
PUBLISHED=ghcr.io/browseros-ai/neo
DEV_IMAGE=neo-dev
export NEO_VNC_PORT=${NEO_VNC_PORT:-16080}
export NEO_MCP_PORT=${NEO_MCP_PORT:-19010}
export NEO_HOST_PORT=${NEO_HOST_PORT:-19300}
NEO_GATEWAY_PORT=${NEO_GATEWAY_PORT:-16090}
# Exported for compose.sdk-dev.yaml, which mounts the host token from here.
export NEO_DEV_HOME=${NEO_DEV_HOME:-$HOME/.local/state/neo-dev}
export NEO_DATA=$NEO_DEV_HOME/data
HOST_TOKEN=$NEO_DEV_HOME/host-token
HOST_URL=http://127.0.0.1:$NEO_HOST_PORT
TRACES=$NEO_DEV_HOME/traces
GATEWAY_PID=$NEO_DEV_HOME/gateway.pid
GATEWAY_LOG=$NEO_DEV_HOME/gateway.log
BUN_VERSION=1.4.2
# The release workflow's server job builds with this toolchain (release-docker.yml pins
# dtolnay/rust-toolchain@1.98.0), so the dev image runs the same compiler's output. Pinning it
# here also keeps `up` incremental: a build on another toolchain recompiles every crate.
RUST_TOOLCHAIN=${NEO_RUST_TOOLCHAIN-1.98.0}

compose() { docker compose -p neo-dev -f "$HERE/compose.yaml" "$@"; }
die() { echo "dev.sh: $*" >&2; exit 1; }
need() { command -v "$1" >/dev/null || die "needs $1; see Setup in this script's header"; }

# The servers `up` builds: kind -> cargo package and binary. The image tells them
# apart by the version marker each binary carries (neo-server-info), not by name.
server_crate() {
  case $1 in
    claw-server) SERVER_PACKAGE=claw-server SERVER_BIN=browseros-claw-server-rs ;;
    claw-sdk) SERVER_PACKAGE=claw-sdk SERVER_BIN=browseros-claw-sdk ;;
    *) die "--server takes claw-server or claw-sdk, not '$1'" ;;
  esac
}

# Which server the running container has. Read from the binary's marker every
# time, never stored, so down/smoke can never disagree with what actually runs.
server_kind() { compose exec -T neo neo-server-info kind; }

# Image tags are <browser>-<commit>; the Dockerfile wants just <browser>.
browser_version() {
  if [ -n "${NEO_BROWSER_VERSION:-}" ]; then echo "$NEO_BROWSER_VERSION"; return; fi
  local version
  version=$(docker buildx imagetools inspect "$PUBLISHED:latest" --format '{{json .Image}}' \
    | jq -r --arg p "linux/$1" '.[$p].config.Labels["org.opencontainers.image.version"] // empty')
  [ -n "$version" ] || die "can't read the browser version of $PUBLISHED:latest; set NEO_BROWSER_VERSION"
  echo "${version%%-*}"
}

# Same cross build as the release workflow's server job (glibc 2.17 floor via
# zig), in its own target dir so the host build's cache isn't invalidated.
# Without CLAW_POSTHOG_KEY the binary has no analytics key; the image turns
# analytics off anyway. The Dockerfile installs <dir>/<arch>/neo-server over the
# bundled server; the name is neutral because the binary's version marker says
# which server it is.
build_server() {
  local arch=$1 target
  case $arch in
    arm64) target=aarch64-unknown-linux-gnu ;;
    amd64) target=x86_64-unknown-linux-gnu ;;
    *) die "unsupported Docker arch $arch" ;;
  esac
  local cargo=(cargo)
  if [ -n "$RUST_TOOLCHAIN" ]; then
    rustup run "$RUST_TOOLCHAIN" rustc --print target-libdir --target "$target" >/dev/null 2>&1 ||
      die "needs Rust $RUST_TOOLCHAIN with $target: rustup toolchain install $RUST_TOOLCHAIN && rustup +$RUST_TOOLCHAIN target add $target (or NEO_RUST_TOOLCHAIN= for your default)"
    cargo=(cargo "+$RUST_TOOLCHAIN")
  fi
  (cd "$AGENT_DIR" && CARGO_TARGET_DIR=target/neo-docker \
    "${cargo[@]}" zigbuild --release --locked --target "$target.2.17" -p "$SERVER_PACKAGE" --bin "$SERVER_BIN")
  install -d "$NEO_DEV_HOME/server/$arch"
  install -m 0755 "$AGENT_DIR/target/neo-docker/$target/release/$SERVER_BIN" "$NEO_DEV_HOME/server/$arch/neo-server"
}

up() {
  local image=$DEV_IMAGE tag=local kind=claw-server
  case ${1:-} in
    --published) image=$PUBLISHED tag=${2:-latest} ;;
    --server) kind=${2:-} ;;
    '') ;;
    *) die "usage: up [--server claw-server|claw-sdk] | up --published [tag]" ;;
  esac
  if [ "$image" = "$PUBLISHED" ]; then
    docker pull "$image:$tag"
  else
    # The daemon's arch, not the host's: it decides which binary the image needs.
    local arch browser
    server_crate "$kind"
    need jq; need zig; need cargo-zigbuild
    [ "$kind" = claw-server ] || { need npx; need openssl; }
    arch=$(docker version --format '{{.Server.Arch}}')
    browser=$(browser_version "$arch")
    build_server "$arch"
    # The neo-server context replaces the package's bundled server, exactly as a
    # release does; the browser package layer stays cached across rebuilds.
    docker build --build-arg NEO_VERSION="$browser" \
      --build-context neo-server="$NEO_DEV_HOME/server" -t "$DEV_IMAGE:local" "$HERE"
  fi
  mkdir -p "$NEO_DATA"
  if [ "$kind" = claw-sdk ]; then
    up_sdk "$image" "$tag"
    return
  fi
  # The gateway only fronts claw-sdk's host API, which this container lacks.
  stop_gateway
  NEO_IMAGE=$image NEO_VERSION=$tag compose up -d --wait
  echo
  echo "Viewer: http://localhost:$NEO_VNC_PORT"
  echo "MCP:    http://localhost:$NEO_MCP_PORT/mcp"
}

up_sdk() {
  ensure_host_token
  NEO_IMAGE=$1 NEO_VERSION=$2 compose -f "$HERE/compose.sdk-dev.yaml" up -d --wait
  local running
  running=$(server_kind)
  [ "$running" = claw-sdk ] || die "built claw-sdk, but the container runs $running"
  copy_novnc
  start_gateway
  local mcp_url=http://127.0.0.1:$NEO_MCP_PORT/mcp
  echo
  echo "Server:   claw-sdk (read from the container)"
  gateway_urls
  echo "MCP:      $mcp_url"
  echo "Host API: $HOST_URL/v1   bearer token in $HOST_TOKEN"
  echo
  echo "MCP client config:"
  echo "  {\"mcpServers\": {\"neo-sdk\": {\"type\": \"http\", \"url\": \"$mcp_url\"}}}"
  echo "Claude Code:"
  echo "  claude mcp add --transport http neo-sdk $mcp_url"
}

# The host API's bearer token. Whoever holds it can read every trace and take
# over the browser, so it is made once, owner-only, and never printed: every
# command (gateway, host, run, export) reads it from this file instead.
# Written to a temp file and renamed so an interrupted run never leaves a short
# token that claw-sdk would refuse.
ensure_host_token() {
  [ -s "$HOST_TOKEN" ] && return
  mkdir -p "$NEO_DEV_HOME"
  (umask 077 && openssl rand -hex 32 >"$HOST_TOKEN.tmp" && mv "$HOST_TOKEN.tmp" "$HOST_TOKEN")
}

# The viewer page loads noVNC's RFB client from the image itself (Ubuntu's 1.3.0,
# the client /v1/display was built against), so the kit has no npm dependency.
# `tar -h` dereferences the Debian package's symlinks into plain files the
# gateway can serve. A fresh copy each `up` keeps it matched to the image.
copy_novnc() {
  local dest=$NEO_DEV_HOME/novnc
  rm -rf "$dest.tmp" && mkdir -p "$dest.tmp"
  compose exec -T neo tar -C /usr/share/novnc -chf - . | tar -C "$dest.tmp" -xf -
  rm -rf "$dest" && mv "$dest.tmp" "$dest"
}

# The gateway (devkit/gateway.ts) is the browser's door to the host API: it adds
# the bearer token, so no page ever holds it, and serves the viewer and the trace
# UI. It runs as Bun's own binary, resolved through npx once, so the pid stored
# here is the server itself: killing an npx wrapper can leave the real Bun
# process orphaned on the port. nohup and the redirects detach it from this
# shell, so it outlives `dev.sh` and the terminal.
start_gateway() {
  stop_gateway
  local bun pid
  bun=$(npx -y "bun@$BUN_VERSION" -e 'console.log(process.execPath)') || die "can't fetch Bun $BUN_VERSION with npx"
  mkdir -p "$TRACES"
  nohup "$bun" "$HERE/devkit/gateway.ts" --port "$NEO_GATEWAY_PORT" --upstream "$HOST_URL" \
    --token-file "$HOST_TOKEN" --novnc-dir "$NEO_DEV_HOME/novnc" --traces-dir "$TRACES" \
    </dev/null >"$GATEWAY_LOG" 2>&1 &
  pid=$!
  echo "$pid" >"$GATEWAY_PID"
  # Ready means this process printed its listening line and the port answers,
  # so an older gateway still holding the port can't pass for the new one.
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

# A pid file can outlive its process (a crash, a reboot) and the pid be reused,
# so only a process that is still running gateway.ts gets signalled.
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

need_token() { [ -s "$HOST_TOKEN" ] || die "no host token at $HOST_TOKEN; run \`dev.sh up --server claw-sdk\`"; }

# The host API from the shell. The token reaches curl as config on stdin, not
# as an argument, so it never shows in `ps` or in shell history.
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
    # Screenshots are saved like dev_mcp.py's images, and the path printed.
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

mcp() { python3 "$HERE/dev_mcp.py" --url "http://localhost:$NEO_MCP_PORT/mcp" --state "$NEO_DEV_HOME" "$@"; }

run() {
  # The host args only add the run's tabs to the status line, so a container
  # without the host API (claw-server) still runs.
  local host_args=()
  [ -s "$HOST_TOKEN" ] && host_args=(--host-url "$HOST_URL" --token-file "$HOST_TOKEN")
  mcp run --traces-dir "$TRACES" ${host_args[@]+"${host_args[@]}"} "$@"
}

smoke() {
  local kind
  kind=$(server_kind)
  case $kind in
    claw-sdk)
      python3 "$HERE/smoke_sdk.py" --mcp-port "$NEO_MCP_PORT"
      compose exec -T neo python3 - claw-sdk <"$HERE/check_display.py"
      ;;
    *) python3 "$HERE/smoke.py" --vnc-port "$NEO_VNC_PORT" --mcp-port "$NEO_MCP_PORT" ;;
  esac
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
