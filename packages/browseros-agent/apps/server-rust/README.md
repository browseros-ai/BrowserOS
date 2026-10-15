# server-rust

`apps/server-rust` is the Rust port of `apps/server` and the server BrowserOS ships: same binary
contract (`--version`, `--config <sidecar>`, exit codes 0/1/2/3), same routes, bodies, status codes
and SSE bytes, same `~/.browseros` data. Browser tools are shared with claw-server through `crates/browseros-mcp-host`.
Package `server-rust`, binary `browseros-server-rs`, staged as `resources/bin/browseros_server`.

```
apps/app ──server-api-client──► apps/server-rust (axum) ──► chat::ChatService ─► ChatEngine
                                   │                         ├─ BrowserOsEngine → browseros-agent (rig) ─┐
                                   │                         └─ AcpEngine → acp-agents → acp-client ─────┤ loopback /mcp
                                   └─ /mcp ◄─────────────── X-BrowserOS-Internal-Lease ◄─────────────────┘
                                         browseros-mcp-host (shared with claw-server) → browseros-mcp → core → cdp
```

Both chat engines speak one vocabulary, `UiChunk` (`crates/ui-message-stream`), and both reach
browser tools the same way: an MCP client over loopback `/mcp` carrying a lease minted per
conversation, so admission, read-only mode and tab ownership are enforced in one place.

## Run

```sh
cargo run -p server-rust --bin browseros-server-rs -- --version
cargo run -p server-rust --bin browseros-server-rs -- --config ../../config.dev.json
tools/dev/run.sh watch --new     # from packages/browseros-agent: BrowserOS + extension + this server
bun run build:server:test        # darwin-arm64 resources zip
```

## Shipping

This is the BrowserOS server release. Release component `server` versions this crate's
`Cargo.toml` (0.1.x, above every TypeScript `agent-server/v0.0.x` tag) under the same tag prefix,
archive names (`browseros-server-resources-<target>.zip`) and R2 prefix (`artifacts/server`), so
Chromium's download, signing and OTA paths are unchanged. `release-server.yml` builds all five
targets with `bun scripts/build/server-rust.ts --target=all --upload --versioned-only` on one
macOS runner; source-mode browser builds use the same packager (macOS hosts only). `bun run
build:server*` and `tools/dev/run.sh watch` default to it.

The TypeScript server stays in `apps/server` and still builds: `bun run build:server-ts:test`,
`tools/dev/run.sh watch --ts`. To ship it again, revert the three flip commits ("ship the Rust
server as the BrowserOS server component", "release the BrowserOS server from the Rust
packager", "default the dev loop and build scripts to the Rust server"); the first two alone
restore the release path. Allocation never goes backwards, so the next TypeScript release is
numbered above the newest published Rust version (0.0.168 after 0.1.3 releases as 0.1.4).

## Verify

The HTTP contract is `contracts/server-api` (OpenAPI). `tests/contract_routes.rs` fails if the
route table drifts from it; `tests/dependency_boundaries.rs` fails if the crate graph breaks a seam.
Parity with the TypeScript server is checked against a real BrowserOS by the compatibility harness:

```sh
cargo test -p server-rust
bun run tools/server-compat/http-compat.ts   --a ts --b rust
bun run tools/server-compat/stream-compat.ts --a ts --b rust
```

## Module map

| Module | Role |
|---|---|
| `main`, `config`, `paths`, `identity`, `activity`, `server_file`, `app` | process contract, sidecar config, data dir, `/status` activity, composition root |
| `http::{mod, security, extract, errors, system}`, `http::wire` | the single `ROUTES` table, request trust (origin/loopback), zod-compatible extractors, error dialects |
| `db` | the shared SQLite file: drizzle catch-up that keeps TS rollback possible, sea-orm stores, backup and corruption salvage |
| `chat::{mod, runs, history, request, engine}`, `http::chat` | `ChatService`, the run registry (409/replay/stop/panels), append-only history, the `ChatEngine` seam |
| `chat::browseros_engine` | built-in agent adapter over `browseros-agent` |
| `chat::acp_engine`, `http::acpx_probe` | external agents (Claude Code, Codex, custom) over `acp-agents` |
| `http::{conversations, agents, providers, schedules}` | CRUD routes |
| `oauth`, `http::oauth` | ChatGPT/Copilot/Qwen OAuth and the `:1455` callback listener |
| `klavis`, `mcp_manager`, `http::{klavis, mcp_manager}` | managed connectors and third-party MCP installs |
| `http::llm_tools`, `telemetry` | test-provider, refine-prompt, Sentry and PostHog |
| `mcp`, `http::mcp` | tool leases and the server's adapters at `browseros-mcp-host`'s seams |

Design decisions and their reasoning live in the overnight decision record (`DECISIONS.md` in the
project's `.llm/server-rust/design/`).
