# Run the Rust server in a private BrowserOS

From `packages/browseros-agent`, use Node.js 22 or later, rustup, and an installed macOS BrowserOS with bundled server resources. The default executable is in `~/My Applications/BrowserOS.app`. Set `BROWSEROS_E2E_BROWSER` to the BrowserOS executable if it is installed elsewhere.

Choose a new run directory outside the repository. Keep the same path for `status` and `down`.

```sh
run_dir="${TMPDIR:-/tmp}/browseros-e2e-$(date +%s)"
node tools/e2e-server/run.mjs up "$run_dir"
node tools/e2e-server/run.mjs status "$run_dir"
node tools/e2e-server/run.mjs down "$run_dir"
```

After `up`, open **Ask BrowserOS** in the new window. Send a message to **BrowserOS E2E Mock**. To run a scripted tool call, pass a mock JSON file as the last argument to `up`. The mock is loaded when the Rust server starts.

The run directory holds the browser profile, server state, launch receipt, and logs. A private loopback bridge forwards the server's CDP connection to the debugger owned by this browser. `status` reports the HTTP port from Chromium's launch file. `down` stops the private browser, server, and CDP bridge. The run directory stays available for inspection.
