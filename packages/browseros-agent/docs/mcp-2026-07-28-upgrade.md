# MCP 2026-07-28 upgrade design

Status: design (Phase 0 of the upgrade epic). No behavior change in this document; it records the verified upstream contract and the migration approach for both MCP server surfaces so later phases can proceed with confidence.

## Summary

BrowserOS exposes two independent MCP servers, and both currently speak only the legacy (`initialize`-handshake) protocol era, so modern MCP clients on revision `2026-07-28` (per-request `_meta`, `server/discover`, stateless) cannot connect. This design brings both onto `2026-07-28` while keeping legacy clients working, by following the official upgrade guides for each SDK. The interim error-handling fix (proper JSON-RPC / HTTP errors instead of a masked 500) already shipped in #2260; this is the follow-up that adds actual modern-protocol support. Context: #2168.

- TS agent server: `apps/server` (Hono + `@modelcontextprotocol/sdk` 1.x + `@hono/mcp`), endpoint `POST /mcp`.
- Rust claw-server: `apps/claw-server-rust` (axum + `rmcp` 2.1.0), endpoint `/mcp` (streamable HTTP) plus stdio, advertised to external plugins via a runtime file.

## Verified upstream contract (Phase 0 findings)

Every fact below was checked against the actual published packages / source, not inferred. Sources are listed at the end.

### TypeScript SDK v2 (scoped packages, stable `2.0.0`)

- The v1 monolith `@modelcontextprotocol/sdk` is superseded by scoped packages. For servers: `@modelcontextprotocol/server`, `@modelcontextprotocol/core`, `@modelcontextprotocol/client`, and the framework adapter `@modelcontextprotocol/hono`. Install: `bun add @modelcontextprotocol/server @modelcontextprotocol/hono @modelcontextprotocol/client hono`. A `@modelcontextprotocol/codemod` package provides v1-to-v2 transforms.
- `@modelcontextprotocol/server` exports (verified in its `.d.ts`): `McpServer`, `Server` (low-level), `WebStandardStreamableHTTPServerTransport` (+ options), `createMcpHandler` (+ `CreateMcpHandlerOptions`, `McpServerFactory`), `isLegacyRequest`, `legacyStatelessFallback`, `ServerOptions`, `CacheHint`, `CacheScope`, `DiscoverRequest` / `DiscoverResult`, `InputRequiredResult` / `inputRequired`, and the host-validation helpers `validateHostHeader` / `validateOriginHeader` / `localhostAllowedHostnames`. `SetLevelRequest` / `SetLevelRequestParams` are still exported for the legacy path.
- `McpServer.registerTool` keeps a `ZodRawShape` overload: `registerTool<InputArgs extends ZodRawShape, ...>(name, { inputSchema, outputSchema?, annotations?, description }, handler)`, alongside a `StandardSchemaWithJSON` overload. This matters because the current browser-tools registration passes a Zod object's `.shape` (a `ZodRawShape`) as `inputSchema`; that pattern is still valid in v2, so the three `registerTool` call sites do not need a schema-shape rewrite.
- Dual-era serving is real and built in: `createMcpHandler(factory, options)` where `options` (`CreateMcpHandlerOptions`) selects the legacy behavior (the default serves both eras statelessly per request; a `reject` mode refuses legacy). The SDK answers `server/discover`, consumes/produces `resultType`, stamps `_meta.serverInfo`, and emits `ttlMs` / `cacheScope` cache hints automatically; handlers do not implement these.
- `@modelcontextprotocol/hono` (verified exports): `createMcpHonoApp(options?)`, `hostHeaderValidation(allowedHostnames)`, `localhostHostValidation()`, `localhostOriginValidation`, `originValidation`. It is a thin layer: a Hono app with MCP-friendly defaults, JSON body parsing exposed as `c.get('parsedBody')`, and DNS-rebinding protection via Host-header validation. Its documented mount pattern is `app.all('/mcp', c => transport.handleRequest(c.req.raw, { parsedBody: c.get('parsedBody') }))` with `new WebStandardStreamableHTTPServerTransport({ sessionIdGenerator: undefined })`.

### Rust SDK `rmcp` 3.x (stable, latest `3.1.2`)

- `rmcp` 3.x adds stable `2026-07-28` support with backward compatibility. MSRV is 1.88. The workspace already declares `rust-version = "1.94"` (`Cargo.toml`), so no toolchain change is needed for the bump.
- Dependency compatibility (verified against `rmcp` 3.1.2's manifest): it uses `schemars ^1.1` (feature `chrono04`), and the workspace already resolves `schemars 1.2.1`, which is semver-compatible, so no forced `schemars` major bump. It targets `axum 0.8` (matches the workspace) and `http 1` (matches); `sse-stream` moves to `0.2.4` (a compatible minor bump from the current `0.2.3`).
- `ProtocolVersion` (verified in `rmcp` source): consts `V_2026_07_28` and `V_2025_11_25` both exist; `LATEST = V_2025_11_25`; `Default = LATEST`; `STANDARD_HEADERS = V_2026_07_28`; `KNOWN_VERSIONS` includes `V_2026_07_28`.
- Crucial for the claw-server: the `ServerHandler` trait's default `supported_protocol_versions()` returns `KNOWN_VERSIONS` (which includes `2026-07-28`), and the trait ships a default `discover()`. The claw-server's `ClawMcpService` has no protocol-version code and does not override these, so on the `rmcp` 3.x bump it inherits modern version negotiation and `server/discover` by default. Implementing discovery/negotiation is therefore not the work; adapting the breaking APIs and the session model is.
- Breaking changes to adapt (from the official 2.x-to-3.x guide): handler return types become MRTR-aware (`call_tool` returns `CallToolResponse`, wrap successes with `.into()`); `with_stateful_mode` is renamed `with_legacy_session_mode` (and `2026-07-28` is always stateless regardless); the metadata type split (`MetaObject` / `RequestMetaObject` / `NotificationMetaObject`); `Annotations.last_modified` becomes `Option<String>`; `structured_content` becomes `Option<Value>`; `logging` / `set_level`, `ping`, and `roots/list_changed` are removed under the modern era (retained only for legacy sessions).

## Track A: apps/server (TypeScript) v1 -> v2

Approach: swap the SDK line and the transport together (the current `@hono/mcp` 0.3.1 peer-pins `@modelcontextprotocol/sdk ^1.29`, so it cannot coexist with v2), keep the per-request server construction (the GHSA-345p-7cg4-v4c7 pattern) and the #2260 error handling, and enable dual-era serving.

| Concern | Current (v1) | Target (v2) |
|---|---|---|
| Server package | `@modelcontextprotocol/sdk` 1.x | `@modelcontextprotocol/server` 2.0.0 |
| Hono transport | `@hono/mcp` `StreamableHTTPTransport`, `handleRequest(c)` | `@modelcontextprotocol/hono` + `WebStandardStreamableHTTPServerTransport`, `handleRequest(c.req.raw, { parsedBody })` |
| Server ctor | `new McpServer({name,title,version}, { capabilities: { logging: {} }, instructions })` | `new McpServer({ name, version, ... })`, drop `logging` for modern |
| SetLevel handler | `server.server.setRequestHandler(SetLevelRequestSchema, ...)` | remove for modern; legacy-only if a consumer needs it |
| Tool registration | `registerTool(name, { inputSchema: zodShape, ... }, handler)` (behind `as unknown as` casts) | same call shape, real types (the `ZodRawShape` overload persists); drop the casts |
| Dual-era | none | `createMcpHandler(factory, { legacy: 'stateless' })`, or the bare `WebStandard` transport serving both |
| Klavis client | `Client` + `StreamableHTTPClientTransport` from `sdk/client` | `@modelcontextprotocol/client` 2.0.0 |

Also covered: the second in-process MCP server for filesystem tools (`apps/server/src/tools/filesystem/register-mcp.ts`) uses the same high-level `registerTool` pattern and gets the identical treatment.

Best-practice hardening for this endpoint: adopt `localhostHostValidation()` (DNS-rebinding protection) and reconsider the current `0.0.0.0` bind for the MCP route.

## Track B: apps/claw-server-rust (Rust) rmcp 2.1 -> 3.x

Approach: bump `rmcp` to 3.x in isolation first (dependency + lockfile + compile-only handler changes, no behavior change), then adopt dual-era behavior. Because the default `ServerHandler` already advertises and negotiates `2026-07-28`, most of the modern-protocol behavior arrives with the bump.

Mechanical adaptations: change `ClawMcpService::call_tool` to return `CallToolResponse` (wrap `CallToolResult` with `.into()`); reconcile the model-type changes where touched (`ToolAnnotations`, `structured_content`, metadata split); keep `Tool::new` / `.with_raw_output_schema` / `.with_annotations` and the schemars-generated `Arc<JsonObject>` schemas (schemars stays compatible). The deprecated `set_level` / `enable_logging` surface lives only in the reference `BrowserMcpService` in the `browseros-mcp` crate (guarded by `#[allow(deprecated)]`) and must be dropped or legacy-gated; the production `ClawMcpService` has no `set_level`.

The one design decision that is not mechanical: the session model. Today the claw-server derives per-connection state (window-group retention) from the `mcp-session-id` header via `LocalSessionManager` and `initialize` / `on_initialized`. SEP-2567 removes protocol-level sessions for modern clients; there is no `mcp-session-id`, and cross-call state must be carried as explicit, server-minted handles passed as ordinary tool arguments. Recommended first landing: dual-era. Keep legacy sessions working for legacy clients (via `legacy_session_mode`), and for modern clients anchor session-scoped state on a server-minted handle. The existing `name_session` tool is a natural place to mint and return that handle, with modern session-scoped tool calls passing it as an argument. This preserves legacy behavior exactly and adds a modern-safe path, without a full stateless rewrite.

## Best practices to adopt (both servers)

- Serve dual-era by default; do not reject legacy unless a product decision says so.
- Let the SDK own the wire envelope: `server/discover`, `resultType`, `ttlMs` / `cacheScope`, `_meta.serverInfo`, and the renumbered error codes (for example resource-not-found is now `-32602`).
- Keep `tools/list` deterministically ordered (both catalogs already are) for client and prompt caching.
- DNS-rebinding / Host-header validation on the localhost endpoints (TS: `localhostHostValidation()`; Rust: the existing browser-origin 403 in `mcp_request_hygiene`, plus a Host allow-list if warranted).
- Keep per-request server instances with no shared mutable state.
- Do not emit log notifications unless a request set a log level.

## Testing and validation

- Dual-client acceptance: a legacy client (Claude Code) connects via `initialize` exactly as today; a modern client (or the MCP Inspector in `2026-07-28` mode) connects via `server/discover` plus per-request `_meta`. Both must list every tool and call a representative one (for example `navigate` then `screenshot`).
- Keep the existing suites green across each bump: the TS route tests in `apps/server/tests/api/routes/mcp.test.ts` (extended in #2260) and the Rust serving tests in `crates/browseros-mcp/tests/rmcp_serving.rs`.
- Add a modern-probe regression: a `server/discover` request returns a discover result rather than a method-not-found error.

## Phasing

- Phase 0 (this document): verify the upstream contract and prerequisites. Result: no toolchain change needed (MSRV already satisfied); all API unknowns resolved above.
- Phase 1: TS agent server dual-era migration (server + transport + tool registration, drop modern logging), including the filesystem MCP server.
- Phase 2: TS Klavis client migration to `@modelcontextprotocol/client` 2.0.0.
- Phase 3: claw-server `rmcp` 3.x bump in isolation (deps + lockfile + compile-only), no behavior change.
- Phase 4: claw-server dual-era serving plus the SEP-2567 session-model handling (legacy sessions preserved, modern handles added).
- Phase 5: best-practices hardening (host validation, bind review, cache hints) and optional Subscriptions/Tasks adoption if a feature needs it.

## Open items to confirm during implementation

- TS: whether to standardize on `createMcpHandler({ legacy: 'stateless' })` or the bare `WebStandardStreamableHTTPServerTransport` for the in-app Hono mount (both are available; pick the one that mounts most cleanly alongside the existing `cors` / trusted-origin middleware).
- Rust: whether any behavior depends on `LocalSessionManager` semantics that change once modern requests are stateless, and the exact `with_legacy_session_mode` configuration for the legacy path.
- Product: whether the claw-server keeps legacy MCP sessions indefinitely (dual-era) or moves fully stateless later; this sets Phase 4's scope.

## References

- MCP `2026-07-28` spec changelog: https://modelcontextprotocol.io/specification/2026-07-28/changelog ; versioning: https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning
- TS SDK v2 migration guide: https://ts.sdk.modelcontextprotocol.io/v2/migration/support-2026-07-28
- rmcp 2.x-to-3.x migration guide: https://github.com/modelcontextprotocol/rust-sdk/discussions/969 ; rmcp 3.0.0 release notes: https://github.com/modelcontextprotocol/rust-sdk/releases/tag/rmcp-v3.0.0
- Package sources: `@modelcontextprotocol/*` at https://github.com/modelcontextprotocol/typescript-sdk ; `rmcp` at https://github.com/modelcontextprotocol/rust-sdk
