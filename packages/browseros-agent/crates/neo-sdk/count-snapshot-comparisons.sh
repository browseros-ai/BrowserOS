#!/usr/bin/env bash
# Sums the snapshot comparisons a CI job's harnesses logged, and decides what the count means.
#
#   crates/neo-sdk/count-snapshot-comparisons.sh <log>      run from packages/browseros-agent
#
# While both snapshot engines exist (design 2.3, layer 5), the live and spec jobs set the
# prelude's compare switch (NEO_COMPARE_SNAPSHOTS=1): on the prelude's path every text call is
# held to Rust's text of the same read, and each run appends one line to the log
# (`NEO_SNAPSHOT_COMPARISONS compared= noTime= differed=`, written by neo-playwright's
# js_runner.rs and the spec loader, claw-loader.mjs). What a zero means depends on which engine
# prints the product's text, so the constant decides, as the built prelude the suites run holds
# it (neo-playwright/assets/prelude.js; build.ts --check keeps it equal to snapshot-text.ts):
#
#   SNAPSHOT_TEXT 'prelude'  every text call goes through the comparison, so the job must have
#                            compared at least one and none may differ: a comparison that
#                            silently did not run is not a pass
#   SNAPSHOT_TEXT 'rust'     the prelude's path no longer runs and there is nothing to compare;
#                            the totals are printed, and a difference still fails
#
# Anything else fails, as does an asset that sets it on no line or on two: the constant or its
# line moved, and this script must move with it. The removal of the old renderer (piece 8)
# deletes the constant, the switch and this script.
set -euo pipefail

log=${1:?count-snapshot-comparisons.sh needs the comparisons log}
source=crates/neo-sdk/neo-playwright/assets/prelude.js
engines=$(sed -n 's/^ *var SNAPSHOT_TEXT = "\([a-z]*\)";$/\1/p' "$source")
# Exactly one line sets it; none or two leave the engine unread.
engine=$([ "$(printf '%s\n' "$engines" | grep -c .)" = 1 ] && printf '%s' "$engines" || true)
touch "$log"
totals=$(awk '/^NEO_SNAPSHOT_COMPARISONS/ { runs++; for (i = 2; i <= NF; i++) { split($i, kv, "="); total[kv[1]] += kv[2] } }
  END { printf "runs=%d compared=%d noTime=%d differed=%d", runs, total["compared"], total["noTime"], total["differed"] }' "$log")
line="SNAPSHOT COMPARISON TOTALS $totals (SNAPSHOT_TEXT '$engine')"
echo "$line"
[ -z "${GITHUB_STEP_SUMMARY:-}" ] || echo "$line" >> "$GITHUB_STEP_SUMMARY"

count() { printf '%s\n' "$totals" | tr ' ' '\n' | sed -n "s/^$1=//p"; }
if [ "$(count differed)" -gt 0 ]; then
  echo "count-snapshot-comparisons.sh: $(count differed) snapshot texts differed between the engines" >&2
  exit 1
fi
case "$engine" in
  prelude)
    if [ "$(count compared)" -eq 0 ]; then
      echo "count-snapshot-comparisons.sh: SNAPSHOT_TEXT is 'prelude' and no snapshot was compared: the compare switch did not run" >&2
      exit 1
    fi
    ;;
  rust) ;;
  *)
    echo "count-snapshot-comparisons.sh: could not read SNAPSHOT_TEXT from $source" >&2
    exit 1
    ;;
esac
