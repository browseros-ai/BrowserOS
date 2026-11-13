#!/usr/bin/env bash
set -euo pipefail

# The takeover integration row needs the bundled claw extension. test:browser still owns the
# headless browser, private home/profile, free CDP port and process-group cleanup; this override
# removes only its extension-disable flag, leaving the server and updater disabled.
args=()
for arg in "$@"; do
  if [[ "$arg" != --disable-browseros-extensions ]]; then
    args+=("$arg")
  fi
done

if [[ "$OSTYPE" == darwin* ]]; then
  browser='/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo'
else
  browser=browserclaw
fi
exec "${NEO_TAKEOVER_BROWSER:-$browser}" "${args[@]}"
