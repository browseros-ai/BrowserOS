# server-rust

`apps/server-rust` is the Rust port of `apps/server`: same binary contract (`--version`,
`--config <sidecar>`, exit codes 0/1/2/3), same routes, bodies, status codes and SSE bytes, same
`~/.browseros` data. Browser tools are shared with claw-server through `crates/browseros-mcp-host`.
Package `server-rust`, binary `browseros-server-rs`, staged as `resources/bin/browseros_server`.

```
apps/app ──server-api-client──► apps/server-rust (axum) ──► chat::ChatService ─► ChatEngine
                                   │                         ├─ BrowserOsEngine → browseros-agent (rig) ─┐
                                   │                         └─ AcpEngine → acp-agents → acp-client ─────┤ loopback /mcp
                                   └─ /mcp ◄─────────────── X-BrowserOS-Internal-Lease ◄─────────────────┘
                                         browseros-mcp-host (shared with claw-server) → browseros-mcp → core → cdp
```

## Run

```sh
cargo run -p server-rust -- --version
cargo run -p server-rust -- --config ../../config.dev.json   # from apps/server-rust
```

The HTTP contract is `contracts/server-api` (OpenAPI). `tests/contract_routes.rs` fails if the
route table drifts from it; `tests/dependency_boundaries.rs` fails if the crate graph breaks a seam.

```sh
cargo test -p server-rust --test contract_routes --test dependency_boundaries
```

## Module map

| Module | Role | Owner |
|---|---|---|
| `main`, `config`, `paths`, `identity`, `activity`, `server_file`, `app` | process contract, sidecar, data dir, composition root | P3 |
| `http::{mod, security, extract, errors, system}`, `http::wire` | the single `ROUTES` table, request trust, zod-style extractors, error dialects | P3 |
| `db` | shared SQLite (drizzle catch-up + sea-orm), five stores | P2 |
| `chat::{mod, runs, history, request, engine}`, `http::chat` | `ChatService`, run registry, display history, the `ChatEngine` seam | P8 |
| `chat::browseros_engine` | built-in agent adapter over `browseros-agent` | P14 |
| `chat::acp_engine`, `http::acpx_probe` | ACP adapter over `acp-agents` | P16 |
| `http::{conversations, agents, providers, schedules}` | CRUD routes | P9 |
| `oauth`, `http::oauth` | ChatGPT/Copilot/Qwen OAuth, `:1455` callback listener | P10 |
| `klavis`, `mcp_manager`, `http::{klavis, mcp_manager}` | connectors, third-party MCP installs | P12 |
| `http::llm_tools`, `telemetry::{sentry, metrics}` | test-provider, refine-prompt, telemetry | P13 |
| `mcp`, `http::mcp` | tool leases and adapters at `browseros-mcp-host`'s seams | P15 |

Shared crates: `crates/ui-message-stream` (the `UiChunk`/`UiMessage` wire vocabulary both engines
emit) and `crates/browseros-agent` (the built-in agent loop over rig-core).

In the S0 skeleton every route answers 501 `{"error":"not implemented"}`; the pieces above replace
handler bodies without changing the table.
