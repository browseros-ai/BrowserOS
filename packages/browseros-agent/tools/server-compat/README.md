# Server compatibility harness

Run commands from `packages/browseros-agent`. This harness compares the old TypeScript server with
an independently booted TypeScript or Rust server. A TS-vs-TS run proves the harness and its deterministic
fixtures work; it does **not** prove Rust parity or complete release acceptance.

## Prerequisites and isolation

- Install workspace dependencies: `bun install --frozen-lockfile`. Bun 1.3.6 is available on the build
  machine; the workspace declares 1.4.2. The installed app's `ai@7.0.114` is the validator/reducer oracle.
- BrowserOS executable: default `/Applications/BrowserOS.app/Contents/MacOS/BrowserOS`, or set
  `BROWSEROS_BINARY=/absolute/path/to/executable` (not the `.app` directory).
- Rust comparisons need either `cargo run -p server-rust` to build successfully, or
  `SERVER_RUST_BINARY=/absolute/path/to/browseros-server-rs`. They are selectable before the port is complete.
- Server boot requires actual CDP. TS connects **before** binding HTTP and exits on CDP failure
  (`apps/server/src/main.ts`, recon `http-api.md` §4.4). Missing BrowserOS is a failure with an actionable
  reason, never a successful skip or a simulated browser.

Every boot creates a separate temporary HOME, BROWSEROS_DIR, browser profile, config and execution directory,
uses ephemeral loopback ports, and supplies the same installation identity. BrowserOS's bundled server and
extensions are disabled. Model credentials, owner config roots and a live public MCP URL are not inherited by
the child server. On POSIX each child has a private process group; teardown signals only that group, reaps
its leader and verifies no live members remain. Windows uses PID-scoped `taskkill /T`; Windows runtime
acceptance has not been executed on this macOS machine.

`boot.ts` owns these resources on success, exceptions and SIGINT/SIGTERM; callers always stop the server in
`finally`. Do not attach this harness to a running owner's browser or database. The unit/HTTP/stream suites
use synthetic credentials and do not need real provider keys.

## Commands

```sh
bun test tools/server-compat
bunx @biomejs/biome check tools/server-compat

# Required harness baseline: both runtimes are independently launched TS processes.
bun run tools/server-compat/stream-compat.ts --a ts --b ts
bun run tools/server-compat/http-compat.ts --a ts --b ts

# Actual differential comparison; defaults are --a ts --b rust.
bun run tools/server-compat/stream-compat.ts --a ts --b rust
bun run tools/server-compat/http-compat.ts --a ts --b rust

# Boot/health/teardown smoke, or run the fake upstream on its own.
bun run tools/server-compat/boot.ts --a ts
bun run tools/server-compat/fake-provider.ts
```

Equivalent root workspace scripts are `test:server-compat`, `test:server-compat:streams`,
`test:server-compat:http`, and `test:server-compat:e2e`. Pass `--a` and `--b` through to choose runtimes.

Optional environment settings:

| Variable | Purpose |
| --- | --- |
| `BROWSEROS_BINARY` | BrowserOS executable to launch with an owned temp profile |
| `SERVER_RUST_BINARY` | Prebuilt Rust server; otherwise invoke Cargo |
| `SERVER_COMPAT_OUTPUT` | Artifact directory; otherwise a new temp directory printed at startup |
| `SERVER_COMPAT_KEEP_SANDBOX=1` | Retain temp state and process logs for debugging after teardown |
| `SERVER_COMPAT_BOOT_TIMEOUT_MS` | Health startup timeout; default TS 30 s / Rust 180 s |
| `SERVER_COMPAT_PROVIDER_PORT` | Standalone fake-provider port; default ephemeral |

## Fake model and stream checks

`fake-provider.ts` serves `/v1/chat/completions`, `/v1/responses` and `/v1/messages`. Set the model name to
`plain-text`, `reasoning-text`, `one-tool`, `parallel-tools`, `unknown-tool`, `retry-429`, `mid-stream-error`
or `long-stream`. Provider/tool/block IDs are deterministic. Tool scenarios use the real `suggest_schedule`
sentinel to exercise the agent loop without external side effects. The tool-result follow-up produces text.
Retry state belongs to `X-Compat-Session` + endpoint + scenario, so A cannot consume B's initial 429.

Unit tests use the installed OpenAI-compatible, Responses and Anthropic SDK adapters to decode the fixture
streams, including reasoning, streamed arguments, parallel calls and errors. Stream comparisons currently
insert an **openai-compatible** provider over HTTP and drive all eight scenarios through the real TS/Rust
agent pipeline. Passing synthetic SDK dialect tests does not establish live authentication/provider parity.

For each side the runner saves original POST SSE, active-run reconnect SSE and the persisted conversation
response. It verifies full-prefix replay equals POST bytes, stale run IDs return 409, /status blocks updates
during work and permits updates afterward, and no fake upstream stream survives completion or stop. It also
captures initial `/chat/panels` state and its heartbeat. Assignment changes with real extension tabs are in
the extension checklist, rather than simulated by this browserless chat fixture.

The stop scenario requests `/stop` after exactly four text deltas. **Observed TS behavior:** registry
cancellation closes the reader before the AI SDK publishes its abort chunk; the wire ends with `[DONE]`,
and a first interrupted turn may have no persisted row. We assert the aborted run and upstream cancellation,
retain the exact wire and persistence HTTP status/body, and compare them. We do not insert an abort chunk,
drop one from the other runtime or turn a missing conversation into an empty successful response.

`wire.ts` requires canonical `data: JSON\n\n` framing and a final `[DONE]`. It preserves JSON key order,
chunk order, null vs omitted values, whitespace/escaping and all text/error/tool payloads. It maps only
text/reasoning IDs and tool-call IDs by first occurrence, preserving reference relationships. The two approved
semantic deltas are F8 (only `start.messageId`) and, explicitly in ACP mode, F6 (only the provider-executed,
dynamic `tool-input-error` with the exact known error text). The built-in suite does not enable the ACP delta.

Before normalization, **every** captured chunk crosses the app's own `uiMessageChunkSchema`; the same stream
is consumed by its AI SDK reducer and by C1's generated `readUiMessageStream` (the shared app/scheduler
transport), split into seven-byte segments. A client-rejected chunk fails the run. Expected streamed model
errors remain part of the comparison. SDK deprecation warnings on TS's unknown-tool `rawInput` parts are
upstream diagnostics, not suppressed failures.

Persisted/HTTP JSON uses sorted object keys, correlation-preserving UUID maps, named server-root/URL
substitutions, and numeric clock fields `createdAt`, `updatedAt`, `lastUsedAt`, `lastActiveAt`, `lastMessagedAt`,
`startedAt`, `completedAt`, `timestamp`, `responseTime`. Arrays, absent/null fields, error contents and all
other values remain observable. Clock normalization proves shape/content compatibility, not timestamp units
or timing semantics; DB round-trip/time assertions belong to the persistence piece.

## HTTP checks

`http-compat.ts` first invokes **the existing** `contracts/server-api/tests/live.test.ts` against each booted
server, without copying or weakening its assertions. Nonzero conformance exits fail immediately; logs are
saved per side. It then runs 43 identical deterministic requests, covering provider/default, conversation,
scheduled job/run and ACP agent CRUD, plus malformed JSON (TS's 500), validation/missing fields, invalid UUID,
missing resources, no/forbidden Origin, CORS preflight and negative DELETE /mcp. Compare exact statuses,
selected content/CORS headers and normalized bodies. All expected statuses are pinned, so two equally broken
implementations do not pass by returning the same generic error for a successful CRUD operation.

The route inventory/full schema coverage lives in C1; this corpus supplements it and does not claim to
exercise every OAuth, upstream connector, probe, destructive system or MCP tool operation.

## Real extension checklist

```sh
# Print every release journey, clearly reporting unexecuted ones as SKIP.
bun run tools/server-compat/e2e/driver.ts --a ts --write-template /tmp/compat-evidence.json

# Launch a fresh visible BrowserOS profile with an already-built extension and run the test-ui inspector.
bun run tools/server-compat/e2e/driver.ts --a rust --extension /absolute/path/to/chrome-mv3

# Incorporate completed human-run journeys, requiring actual evidence files on both observable sides.
bun run tools/server-compat/e2e/driver.ts --a rust \
  --extension /absolute/path/to/chrome-mv3 --evidence /tmp/compat-evidence.json --required
```

The driver uses `scripts/dev/inspect-ui.ts` from the **test-ui skill**: targets, accessibility snapshot,
and screenshot over this run's CDP port. It never reuses element IDs after a render or connects to an owner
browser. It stops its owned browser/server after inspection. For interactive journeys, run the isolated
`tools/dev/run.sh watch --new` (add `--rust` for Rust), preserve its temporary HOME/profile, and follow the
instructions in `e2e/checklist.ts` with `BROWSEROS_CDP_PORT` set to that run's printed port. Supervision/exit-code
journeys specifically require that dev supervisor: the boot helper disables the bundled server supervisor.
Never install MCP configs, OAuth credentials or connectors into an owner's HOME as part of these checks.

Fill in the evidence template after executing each step: `status`, concrete `reason`, `uiEvidence` and
`serverEvidence` paths (relative to the evidence JSON or absolute). PASS entries are labeled **manual evidence**;
the driver checks that both files exist and that runtime/platform match, not that a human's assertions are
true. Keep each journey's UI screenshot/snapshot and relevant server response/process receipt together.

Every journey prints PASS/FAIL/SKIP with a reason. Any FAIL exits nonzero. Default checklist-only mode can
exit zero with explicit SKIPs; `--required` fails on any SKIP or absent extension launch. This is deliberate:
a printed checklist is useful overnight, and is never reported as completed end-to-end acceptance. Real-key
chat, ACP CLIs, OAuth, connector credentials, supervision and macOS/Windows release journeys require separate
live evidence. DB fixture/rollback work belongs to `make-db-fixture.ts` (P2), and the golden-agent work belongs
to `golden-agent.ts` (P6); this harness does not own those files.

## What green means

The unit gate validates the oracle and scripted upstreams. A zero stream-comparison exit requires both
servers to boot, all eight scenarios to pass, all SDK/client parses to succeed, POST/replay to agree,
normalized wire/persistence/header comparisons to agree, and cleanup to complete. A zero HTTP-comparison
exit requires C1 live conformance on both and all 43 differential requests. No unavailable runtime or failed
required check is silently skipped. Full release acceptance additionally requires the real extension's
`--required` checklist, live provider/ACP evidence and the other pieces' DB/MCP/platform gates.
