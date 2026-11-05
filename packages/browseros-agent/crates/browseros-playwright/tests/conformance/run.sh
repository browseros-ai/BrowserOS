#!/usr/bin/env bash
# Conformance (design D15). `run.sh tier1 [case ...]` runs every case in cases/ (or the named
# ones) on real Playwright (runner-p.mjs) and on Neo (tests/conformance_live.rs), diffs the two
# (diff.mjs) against deviations.yaml, and exits 0 only when every difference is listed.
# harness.js says how to write a case. `run.sh tier2 [case ...]` does the same for the cases
# tier2/convert.mjs lifts from upstream's own tests (cases/tier2, after checking they are
# fresh), against tier2/deviations.yaml.
#
# It starts what it needs and stops it at exit: the fixture server (server.mjs) and, unless
# BROWSEROS_CDP_PORT names a running one, a throwaway headless BrowserOS with its own profile.
#
#   PLAYWRIGHT_NODE_MODULES  node_modules holding playwright-core and @playwright/test at the
#                            vendored version (required; the sdk-playwright spikes install one)
#   BROWSEROS_CDP_PORT       a throwaway BrowserOS to use instead of starting one
#   PORT_BASE                the started browser's ports, PORT_BASE to PORT_BASE+3 (9351)
#   BROWSEROS_APP            the BrowserOS binary to start
#   CONFORMANCE_OUT          where outcomes go (packages/browseros-agent/target/conformance),
#                            tier 2's in its tier2/
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
agent="$(cd "$here/../../../.." && pwd)"

out="${CONFORMANCE_OUT:-$agent/target/conformance}"
case "${1:-}" in
  tier1) deviations="$here/deviations.yaml" ;;
  tier2)
    deviations="$here/tier2/deviations.yaml"
    export CONFORMANCE_CASES_DIR=cases/tier2
    out="$out/tier2"
    ;;
  *)
    echo "usage: run.sh tier1|tier2 [case ...]" >&2
    exit 2
    ;;
esac
tier="$1"
shift
: "${PLAYWRIGHT_NODE_MODULES:?set PLAYWRIGHT_NODE_MODULES to a node_modules with playwright-core}"
export PLAYWRIGHT_NODE_MODULES
if [[ "$tier" == tier2 ]]; then node "$here/tier2/convert.mjs" --check; fi
mkdir -p "$out"
# The logs go too: the server's port is read from its log, and the shell truncates a redirect
# only once the background child runs, so an old log could hand out the last run's port.
rm -rf "$out/p" "$out/n" "$out/server.log" "$out/browser.log"

profile=""
server_pid=""
# Stops only what this run started: its server, and the browser that owns its own profile.
cleanup() {
  if [[ -n "$server_pid" ]]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  if [[ -n "$profile" ]]; then
    pkill -f -- "$profile" 2>/dev/null || true
    for _ in $(seq 1 25); do
      pgrep -f -- "$profile" >/dev/null || break
      sleep 0.2
    done
    rm -rf "$profile"
  fi
  return 0
}
trap cleanup EXIT

# Waits up to 20 s for the command to succeed.
wait_for() {
  for _ in $(seq 1 100); do
    if "$@"; then return 0; fi
    sleep 0.2
  done
  echo "run.sh: timed out waiting for: $*" >&2
  return 1
}
cdp_up() { curl -sf "http://127.0.0.1:$BROWSEROS_CDP_PORT/json/version" >/dev/null; }

if [[ -z "${BROWSEROS_CDP_PORT:-}" ]]; then
  base="${PORT_BASE:-9351}"
  app="${BROWSEROS_APP:-/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo}"
  profile="$(mktemp -d "${TMPDIR:-/tmp}/conformance-profile.XXXXXX")"
  # Hosted mode, as browser.sh starts it, so popups open as tabs.
  BROWSERCLAW_HOSTED_MODE=1 "$app" --headless=new --user-data-dir="$profile" \
    --remote-debugging-port="$base" --browseros-cdp-port="$((base + 1))" \
    --browseros-server-port="$((base + 2))" --browseros-proxy-port="$((base + 3))" \
    --disable-browseros-server --disable-browseros-server-updater --disable-browseros-extensions \
    --no-first-run --no-default-browser-check --window-size=1280,900 about:blank \
    >"$out/browser.log" 2>&1 &
  export BROWSEROS_CDP_PORT="$base"
fi
export BROWSEROS_CDP_PORT
wait_for cdp_up

node "$here/server.mjs" 0 >"$out/server.log" 2>&1 &
server_pid=$!
server_up() { grep -q '^listening ' "$out/server.log"; }
wait_for server_up
CONFORMANCE_PORT="$(sed -n 's/^listening //p' "$out/server.log")"
export CONFORMANCE_PORT

node "$here/runner-p.mjs" "$out/p" "$@"
(
  cd "$agent"
  CONFORMANCE_OUT="$out/n" CONFORMANCE_CASES="$*" \
    cargo +1.98.0 test -p browseros-playwright --locked --test conformance_live -- --ignored --nocapture
)
node "$here/diff.mjs" "$out/p" "$out/n" "$deviations" --stubs
