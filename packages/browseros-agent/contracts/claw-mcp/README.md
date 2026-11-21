# Claw MCP contract suite

The suite drives the production Rust server over MCP with a private BrowserOS
profile and deterministic fixtures. The catalog pass runs with
`CLAW_PLAYWRIGHT_RUN=0` and exercises the desktop tool catalog. The Playwright
pass runs with `CLAW_PLAYWRIGHT_RUN=1` and exercises the Neo SDK's `run` and
the shared human-help tools.

## Run the tests

From `packages/browseros-agent`, set `BROWSEROS_BINARY` to the BrowserOS executable
and run both passes:

```bash
export BROWSEROS_BINARY='/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo'
bun run test:claw-mcp-contract
bun run test:claw-mcp-smoke
```

Run just the pass you are working on:

```bash
bun run test:claw-mcp-contract --surface=catalog
bun run test:claw-mcp-contract --surface=playwright
bun run test:claw-mcp-smoke --surface=playwright
```

Without `BROWSEROS_BINARY`, the suite skips. The runner builds the Rust server
once with the flag unset, then starts a new test process with the flag set for
each pass. Cases within a pass run sequentially. Cases with different browser
arguments or a different headless setting restart their private browser and
server. CI receives one JUnit report containing the results from both passes.

| Variable | Effect |
| --- | --- |
| `BROWSEROS_BINARY` | BrowserOS executable to test |
| `CLAW_MCP_SMOKE=1` | Run only cases marked as smoke. The smoke command sets this through `--smoke`. |
| `CLAW_MCP_ONLY=<text>` | Run cases whose names contain the text in each selected pass. |
| `BROWSEROS_JUNIT_PATH` | Write the combined JUnit report to this path. |
| `BROWSEROS_TEST_HEADLESS=false` | Run headed. |
| `BROWSEROS_TEST_EXTRA_ARGS` | Add browser process arguments. |
| `BROWSEROS_TEST_DEBUG=true` | Stream browser output. |
| `CLAW_MCP_CAPTURE_DIR=<dir>` | Refresh raw CDP capture fixtures. |

On Linux, the repeated screenshot case runs headed. The CI browser wrapper
starts Xvfb for it.

Run fixture-server and MCP-client tests without a browser:

```bash
bun test contracts/claw-mcp/tests/fixture-server.test.ts contracts/claw-mcp/tests/mcp-client.test.ts
```

## Add a case

Add catalog cases to the relevant file under `tests/` and register them in
`tests/cases.ts`. The catalog case order matters. Keep the browser-kill case
last. The runner closes pages opened with `ctx.openPage` between cases.

Add Playwright cases under `tests/playwright/` and register them in
`tests/playwright/cases.ts`. Call `run` with Playwright code and assert the
observable result. The SDK starts fresh JavaScript for each call. To share
tabs across calls, create a task with `neo.createTask`, return its id, and
select it with `neo.task` on the next call. The runner enumerates the private
profile to close tabs, including those left behind by a failed script.

## Capture raw CDP fixtures

```bash
CLAW_MCP_CAPTURE_DIR="$PWD/crates/browseros-core/tests/data/captured" \
  bun run test:claw-mcp-smoke --surface=catalog
```
