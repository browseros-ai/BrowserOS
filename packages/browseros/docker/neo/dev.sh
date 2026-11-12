#!/usr/bin/env bash
#
# Local BrowserOS neo in Docker, running this checkout's claw-server, for testing
# server changes the way hosted users (zuzu, other agent backends) see them.
#
#   ./dev.sh up              Build claw-server from this checkout, bake it into the
#                            image (this folder's Dockerfile), start Neo, print URLs.
#                            Run it again after every server change.
#   ./dev.sh up --published [tag]
#                            Run ghcr.io/browseros-ai/neo:<tag> (default latest)
#                            unchanged, to compare against what is released.
#   ./dev.sh tools [name]    List Neo's MCP tools, or one tool's description and schema.
#   ./dev.sh call <tool> [json-args]
#                            Call one MCP tool and print its result. Images are saved
#                            to files and their paths printed.
#   ./dev.sh logs            Follow the container's logs (entrypoint, browser, server).
#   ./dev.sh smoke           Run smoke.py against this container.
#   ./dev.sh down            Stop the container; the profile and logins are kept.
#   ./dev.sh reset           Stop the container and delete its profile.
#
#   Viewer (noVNC): http://localhost:16080      MCP: http://localhost:19010/mcp
#
# Setup, once: Docker, plus what the release workflow's server job installs to
# cross-build claw-server for Linux:
#   brew install jq zig && cargo install --locked cargo-zigbuild
#   rustup target add aarch64-unknown-linux-gnu    # x86_64-unknown-linux-gnu on Intel
#
# Testing a change (for agents):
#   1. `./dev.sh up`, then give the owner the viewer URL; they watch the agent there.
#   2. Drive Neo only through its own MCP tools via `./dev.sh call`, never Playwright,
#      Puppeteer, raw CDP or a headless browser: the MCP surface is what ships, so a
#      test through anything else proves nothing about it. Read the schemas with
#      `./dev.sh tools <name>` rather than guessing argument names.
#        ./dev.sh call tabs '{"action":"new","url":"https://example.com"}'
#        ./dev.sh call snapshot '{"page":2}'
#        ./dev.sh call screenshot '{"page":2}'     # prints a .png path; read it
#      Prefer `run` (one async JS script against the `browser` SDK) for multi-step flows.
#   3. Calls share one MCP session (dev_mcp.py), so pages opened by one call belong to
#      the next. When Neo drops it (30 idle minutes, a new container), the next call
#      starts a fresh one by itself.
#   4. Check `./dev.sh logs` for server errors, and run `./dev.sh smoke` before handing
#      off. Leave pages open for the owner to inspect.
#   A Claude Code session can also add Neo as an MCP server, though only sessions
#   started afterwards see its tools:
#      claude mcp add --transport http neo-docker http://localhost:19010/mcp
#
# Settings (environment):
#   NEO_VNC_PORT, NEO_MCP_PORT  Host ports (16080, 19010), clear of a desktop Neo's.
#   NEO_DEV_HOME                Profile, staged server, MCP session and saved images
#                               (~/.local/state/neo-dev). Kept outside the repo so
#                               worktrees share one signed-in profile, and so the
#                               profile never enters this folder's Docker build context.
#   NEO_BROWSER_VERSION         Neo browser package `up` builds on (default: the
#                               browser in the published `latest` image).
#
# One dev Neo per machine: `up` from another worktree replaces the running container
# with that worktree's server. The compose project is `neo-dev`; it runs
# compose.yaml unchanged, with the image, ports and data folder set below.

set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
AGENT_DIR=$(cd "$HERE/../../../browseros-agent" && pwd)
PUBLISHED=ghcr.io/browseros-ai/neo
DEV_IMAGE=neo-dev
export NEO_VNC_PORT=${NEO_VNC_PORT:-16080}
export NEO_MCP_PORT=${NEO_MCP_PORT:-19010}
NEO_DEV_HOME=${NEO_DEV_HOME:-$HOME/.local/state/neo-dev}
export NEO_DATA=$NEO_DEV_HOME/data

compose() { docker compose -p neo-dev -f "$HERE/compose.yaml" "$@"; }
die() { echo "dev.sh: $*" >&2; exit 1; }
need() { command -v "$1" >/dev/null || die "needs $1; see Setup in this script's header"; }

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
# analytics off anyway. The Dockerfile reads the result from <dir>/<arch>/.
build_server() {
  local arch=$1 target
  case $arch in
    arm64) target=aarch64-unknown-linux-gnu ;;
    amd64) target=x86_64-unknown-linux-gnu ;;
    *) die "unsupported Docker arch $arch" ;;
  esac
  (cd "$AGENT_DIR" && CARGO_TARGET_DIR=target/neo-docker \
    cargo zigbuild --release --locked --target "$target.2.17" -p claw-server --bin browseros-claw-server-rs)
  install -d "$NEO_DEV_HOME/server/$arch"
  install -m 0755 "$AGENT_DIR/target/neo-docker/$target/release/browseros-claw-server-rs" "$NEO_DEV_HOME/server/$arch/"
}

up() {
  local image tag
  if [ "${1:-}" = --published ]; then
    image=$PUBLISHED tag=${2:-latest}
    docker pull "$image:$tag"
  else
    # The daemon's arch, not the host's: it decides which binary the image needs.
    local arch browser
    need jq; need zig; need cargo-zigbuild
    arch=$(docker version --format '{{.Server.Arch}}')
    browser=$(browser_version "$arch")
    build_server "$arch"
    # The neo-server context replaces the package's bundled server, exactly as a
    # release does; the browser package layer stays cached across rebuilds.
    docker build --build-arg NEO_VERSION="$browser" \
      --build-context neo-server="$NEO_DEV_HOME/server" -t "$DEV_IMAGE:local" "$HERE"
    image=$DEV_IMAGE tag=local
  fi
  mkdir -p "$NEO_DATA"
  NEO_IMAGE=$image NEO_VERSION=$tag compose up -d --wait
  echo
  echo "Viewer: http://localhost:$NEO_VNC_PORT"
  echo "MCP:    http://localhost:$NEO_MCP_PORT/mcp"
}

mcp() { python3 "$HERE/dev_mcp.py" --url "http://localhost:$NEO_MCP_PORT/mcp" --state "$NEO_DEV_HOME" "$@"; }

case ${1:-} in
  up) shift; up "$@" ;;
  tools|call) mcp "$@" ;;
  logs) compose logs -f ;;
  smoke) python3 "$HERE/smoke.py" --vnc-port "$NEO_VNC_PORT" --mcp-port "$NEO_MCP_PORT" ;;
  down) compose down ;;
  reset) compose down; rm -rf "$NEO_DATA" ;;
  *) sed -n '3,/^set -euo/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
