#!/usr/bin/env bash
# Heavy presets use each job's own process tree, browser profile, image tag and reports.
# The guest supervisor owns cancellation and removes any Docker resources left by the job.
set -euo pipefail
action=${1:?missing recipe}; shift
mkdir -p "${REMOTE_BUILD_ARTIFACTS:?guest supervisor must supply an artifact directory}"
case "$action" in
  image)
    platform=linux/amd64
    if [ "$#" = 2 ] && [ "$1" = --platform ]; then platform=$2; shift 2; fi
    [ "$#" = 0 ] || { echo 'image accepts only --platform linux/amd64|linux/arm64' >&2; exit 2; }
    case "$platform" in linux/amd64 | linux/arm64) ;; *) exit 2 ;; esac
    image="neo-sdk:remote-${REMOTE_BUILD_JOB:?}"
    build_started=$SECONDS
    bash /usr/local/lib/browseros-remote-builds/build-image.sh --platform "$platform" --tag "$image" \
      | tee "$REMOTE_BUILD_ARTIFACTS/image-build.txt"
    build_seconds=$((SECONDS-build_started))
    docker image inspect "$image" --format '{{json .Id}}' \
      | python3 -c 'import json,sys; print(json.dumps({"id":json.load(sys.stdin),"tag":sys.argv[1],"platform":sys.argv[2]}))' "$image" "$platform" \
      > "$REMOTE_BUILD_ARTIFACTS/image.json"
    accept_started=$SECONDS
    bash crates/neo-sdk/neo-server/docker/accept.sh "$image" \
      --prefix "remote-$REMOTE_BUILD_JOB" --logs "$REMOTE_BUILD_ARTIFACTS/accept" \
      | tee "$REMOTE_BUILD_ARTIFACTS/image-accept.txt"
    printf '{"buildSeconds":%s,"acceptSeconds":%s}\n' "$build_seconds" "$((SECONDS-accept_started))" \
      > "$REMOTE_BUILD_ARTIFACTS/image-metrics.json"
    ;;
  spec)
    # Existing worktrees predate the explicit source-metadata API in run.mjs. The
    # preload supplies only that runner's HEAD lookup; public corpus Git is unchanged.
    export NODE_OPTIONS='--require=/usr/local/lib/browseros-remote-builds/source-metadata-compat.cjs'
    # Older worker branches own at most four browsers. Respect that checkout's
    # public cap instead of requiring a source update before it can use the fleet.
    workers=$(node --input-type=module -e 'import * as o from "./crates/neo-sdk/neo-playwright-tests/node/options.mjs"; console.log(Math.min(8, o.MAX_WORKERS || 4))')
    # Preserve the checkout's CLI and exit status. Callers can request --ci on
    # branches that provide the baseline verdict; legacy runners lack that flag.
    bun run test:neo-spec --workers "$workers" "$@"
    ;;
  detector)
    if node -e 'process.exit(JSON.parse(require("fs").readFileSync("package.json")).scripts?.["test:browser"] ? 0 : 1)'; then
      # Current checkouts already own browser lifecycle and cancellation. Reuse
      # that contract, including its Linux executable selection and private state.
      result=0
      bun run test:browser -- cargo +1.98.0 test -p neo-playwright --locked \
        --test devtools_detection_live -- --ignored --nocapture --test-threads=1 "$@" \
        2>&1 | tee "$REMOTE_BUILD_ARTIFACTS/detector.txt" || result=$?
      for run in .llm/test-browser/run-*; do
        [ -d "$run" ] || continue
        destination="$REMOTE_BUILD_ARTIFACTS/browser/$(basename "$run")"
        mkdir -p "$destination"
        for file in result.json owner.json browser.log; do
          [ ! -f "$run/$file" ] || cp "$run/$file" "$destination/$file"
        done
      done
      exit "$result"
    fi
    # Compatibility for pre-launcher branches: only this fresh profile/process is
    # started, and systemd remains the outer cleanup boundary for all descendants.
    profile=$(mktemp -d)
    browser_pid=''
    cleanup() {
      if [ -n "$browser_pid" ]; then kill "$browser_pid" 2>/dev/null || true; wait "$browser_pid" 2>/dev/null || true; fi
      rm -rf "$profile"
    }
    trap cleanup EXIT
    /usr/bin/browserclaw --headless=new --no-first-run --no-default-browser-check \
      --disable-browseros-server --disable-browseros-extensions --lang=en-US \
      --remote-debugging-port=0 --user-data-dir="$profile" about:blank \
      >"$REMOTE_BUILD_ARTIFACTS/browser.log" 2>&1 &
    browser_pid=$!
    for _ in $(seq 1 100); do
      [ ! -s "$profile/DevToolsActivePort" ] || break
      kill -0 "$browser_pid" || { echo 'owned browser exited before CDP was ready' >&2; exit 1; }
      sleep 0.2
    done
    BROWSEROS_CDP_PORT=$(head -1 "$profile/DevToolsActivePort")
    export BROWSEROS_CDP_PORT
    cargo +1.98.0 test -p neo-playwright --locked --test devtools_detection_live \
      -- --ignored --nocapture --test-threads=1 "$@" \
      2>&1 | tee "$REMOTE_BUILD_ARTIFACTS/detector.txt"
    ;;
  *) echo "unknown recipe: $action" >&2; exit 2 ;;
esac
