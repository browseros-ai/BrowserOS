# claw-mcp — Rust real-browser conformance suite

Tier-3 integration coverage for the BrowserClaw stack: deterministic fixture
pages, a spawned BrowserOS, and the production Rust Claw server driven over its
real `/mcp` endpoint. A live browser is required because lower-level tests cannot
catch mismatches between CDP payloads and the accessibility/ref pipeline.

## Coverage

The suite covers every MCP tool and major failure path: `tabs`, `tab_groups`,
`windows`, `navigate`, `snapshot`, `diff`, `act`, `grep`, `read`, `evaluate`,
`run`, `screenshot`, `pdf`, `wait`, `upload`, `download`, and `name_session`.
It also checks ownership isolation, untrusted-content fencing, auto-context,
audit records, cancellation, browser-down guidance, and transport hygiene.

Each case asserts Rust behavior directly. The retired TypeScript server and its
historical behavior are not test dependencies.

## Running it

The browser suite is gated on `BROWSEROS_BINARY`; without it, the suite skips
cleanly. From `packages/browseros-agent`:

```bash
BROWSEROS_BINARY=/Applications/BrowserClaw.app/Contents/MacOS/BrowserClaw \
  bun run test:claw-mcp-contract

BROWSEROS_BINARY=/Applications/BrowserClaw.app/Contents/MacOS/BrowserClaw \
  bun run test:claw-mcp-smoke
```

`run.ts` builds the Rust server before entering test timeouts. One isolated
browser profile and server process are shared by the ordered cases, with pages
cleaned between cases.

Useful environment variables:

| Variable | Effect |
| --- | --- |
| `BROWSEROS_BINARY` | BrowserOS or BrowserClaw executable to test |
| `CLAW_MCP_SMOKE=1` | Run only smoke cases; set by `--smoke` |
| `BROWSEROS_TEST_HEADLESS=false` | Run headed for debugging |
| `BROWSEROS_TEST_EXTRA_ARGS` | Add browser process arguments |
| `BROWSEROS_TEST_DEBUG=true` | Stream browser output |
| `CLAW_MCP_CAPTURE_DIR=<dir>` | Refresh raw CDP capture fixtures |

The ungated fixture-server and MCP-client unit tests run under the normal Bun
test suite.

## Layout

```text
contracts/claw-mcp/
  fixtures/                 deterministic pages and a two-origin static server
  tests/browser.ts          isolated real-BrowserOS runtime
  tests/mcp-client.ts       raw streamable-HTTP JSON-RPC/SSE client
  tests/rust-server.ts      production Rust server launcher
  tests/cases-*.ts          behavioral cases grouped by tool surface
  tests/cases.ts            ordered registry; browser-kill stays last
  tests/rust-conformance.test.ts
  tests/run.ts              gated CLI entrypoint
```

## Adding a case

Add it to the relevant `cases-<group>.ts`, assert the expected behavior in the
case, and use `waitUntil` for asynchronous state. Pages opened with
`ctx.openPage` are cleaned automatically. Keep the browser-kill case last.

## Capture mode

`CLAW_MCP_CAPTURE_DIR` writes the raw accessibility, frame-tree, and DOM payloads
used by `browseros-core` serde fixtures:

```bash
CLAW_MCP_CAPTURE_DIR="$PWD/crates/browseros-core/tests/data/captured" \
  BROWSEROS_BINARY=/Applications/BrowserClaw.app/Contents/MacOS/BrowserClaw \
  bun contracts/claw-mcp/tests/run.ts --smoke

cargo test -p browseros-core --test captured_cdp_fixture
```

The full suite runs in the nightly BrowserClaw workflow against the browser built
by that workflow.
