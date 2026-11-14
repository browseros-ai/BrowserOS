# Claw MCP contract suite

The suite drives the production desktop server over MCP with a private BrowserOS
profile. Its browser tool is the Neo SDK's Playwright `run`. Human-help tools are
listed by default. `pursue` requires ultrafast and a configured decisions worker,
which this suite does not start.

From `packages/browseros-agent`, run:

```bash
BROWSEROS_BINARY='/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo' \
  bun run test:claw-mcp-smoke

BROWSEROS_BINARY='/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo' \
  bun run test:claw-mcp-contract
```

Without `BROWSEROS_BINARY`, the suites skip. `run.ts` builds the Rust server before
entering test timeouts. Cases share the suite-owned browser and run sequentially.
Setup, readiness, and tab cleanup also use `run`, so a removed tool cannot make a
case pass. Cleanup enumerates the private profile to reclaim tabs left by failed
scripts.

The smoke tier covers navigation and reads, accessibility refs and clicks,
fill and readback, new tabs, upload payloads, download `saveAs`, assertion errors,
browser-request rejection, and a removed-tool refusal naming `run`. The full
suite adds the remaining Playwright and transport cases. Retired tool handlers
remain covered by `browseros-mcp`'s tests.

| Variable | Effect |
| --- | --- |
| `BROWSEROS_BINARY` | BrowserOS executable to test |
| `CLAW_MCP_SMOKE=1` | Run smoke cases, set by `--smoke` |
| `CLAW_MCP_ONLY=<text>` | Select case names, such as `playwright run:` |
| `BROWSEROS_TEST_HEADLESS=false` | Run headed |
| `BROWSEROS_TEST_EXTRA_ARGS` | Add browser process arguments |
| `BROWSEROS_TEST_DEBUG=true` | Stream browser output |
| `CLAW_MCP_CAPTURE_DIR=<dir>` | Refresh raw CDP capture fixtures |

## Add a case

Add a `ContractCase` to the relevant case module and register its array in
`tests/cases.ts`. Call `run` with Playwright code and assert observable results.
The SDK starts fresh JavaScript for each call. To share tabs across calls, create
a task with `neo.createTask`, return its id, and select it with `neo.task` later.
Do not use the retired `browser.page(id)` dialect or granular MCP tools.

`tests/sdk-helpers.ts` reads `structuredContent.value` from the SDK reply. It does
not add an envelope or use a separate evaluation tool to inspect the page.

## Layout

- `fixtures/server.ts` serves deterministic pages and downloads on two origins.
- `tests/browser.ts` owns the private browser profile and its process.
- `tests/mcp-client.ts` handles JSON and SSE replies over raw HTTP.
- `tests/rust-server.ts` owns the sandboxed production server process.
- `tests/cases.ts` registers the ordered transport and Playwright cases.
- `tests/rust-conformance.test.ts` connects and cleans up those runtimes.

Run the fixture-server and MCP-client tests directly, without a browser:

```bash
bun test contracts/claw-mcp/tests/fixture-server.test.ts contracts/claw-mcp/tests/mcp-client.test.ts
```

The repository-wide test command has global browser cleanup and must not run while
other workers have private browsers open.

## Capture raw CDP fixtures

```bash
CLAW_MCP_CAPTURE_DIR="$PWD/crates/browseros-core/tests/data/captured" \
  BROWSEROS_BINARY='/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo' \
  bun contracts/claw-mcp/tests/run.ts --smoke
```
