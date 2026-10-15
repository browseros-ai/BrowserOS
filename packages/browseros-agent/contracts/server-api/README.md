# BrowserOS server compatibility contract

`openapi.yaml` describes the existing TypeScript server's unversioned HTTP wire
format. The Rust port and extension client share these types; this piece does
not migrate the app or change either server implementation.

Run from `packages/browseros-agent`:

```sh
bun run codegen:server-api
bun run codegen:server-api:check
bun run codegen:claw-api:check
bun run test:server-api-contract
cargo test -p server-api
cargo clippy -p server-api --all-targets -- -D warnings
bunx @biomejs/biome check contracts/server-api packages/server-api-client scripts/codegen
bun run --filter @browseros/server-api-client typecheck
```

The split `paths/` groups preserve status codes and error dialects. `schemas/`
exports named request/response models and the AI SDK v7 `UiMessageChunk` union.
`responses.yaml` shares descriptions without replacing legacy error payloads.
Root `/health` and `/shutdown` aliases and `can_update` remain intact. DELETE
`/mcp` is explicitly an unregistered plain-text 404 on the TypeScript server.
The PKCE callback listener on port 1455 is a separate server.

The generator uses Docker OpenAPI Generator v7.22.0 for models and pinned
openapi-typescript/TypeScript 5 for operations. It bundles split references to
avoid duplicate union models, emits formatted type-only TS unions with unknown for free JSON, preserves Rust opaque
maps and extension fields, and fixes an invalid upstream Rust array-variant
identifier. Nullable request fields use `serde_with::rust::double_option` so
omitted values and explicit null remain distinct. The original claw generator
and templates are unchanged. Generated trees are committed; edit the contract
and regenerate rather than editing output.

## Client

```ts
import { ServerApiClient, readUiMessageStream } from '@browseros/server-api-client'

const api = new ServerApiClient(baseUrl, { fetch })
const { data } = await api.http.GET('/providers')
const response = await api.createChat({ conversationId, message: 'Hello' })
for await (const chunk of readUiMessageStream(response)) {
  // chunk narrows by its `type` field.
}
```

`http` exposes every generated operation with typed paths, bodies and responses.
All HTTP failures throw `ServerApiError { status, code?, message }`, including
plain-text OAuth failures, nested chat errors, Zod errors and message-only
errors. Network/abort errors preserve the fetch implementation's own error.
`createChat` and `reconnectChat` return an unconsumed Response for the AI SDK or
the shared reader; reconnect returns null on 204. The reader validates chunks
with a generated guard, handles fragmented UTF-8 and SSE LF/CRLF/CR framing,
and requires the `[DONE]` sentinel to avoid treating truncated work as complete.
Breaking iteration cancels the local subscriber and releases its reader; it
does not stop the server's run. Use the explicit stop endpoint for that.

## Evidence and live conformance

Every fixture is indexed with its schema and provenance in
`fixtures/manifest.json`. These are source-derived examples, not captured live
responses; credential-looking values are synthetic. AJV validates every fixture,
including negative cases, and Rust deserializes the same complete inventory.
The route index pins every operation, request reference, response status and
response schema; an additional test compares operations with TS registrations.

`live.test.ts` is skipped unless `SERVER_API_BASE_URL` is set:

```sh
SERVER_API_BASE_URL=http://127.0.0.1:9105 bun run test:server-api-contract
```

It validates non-destructive GETs and uniquely named provider, conversation,
disabled job, job-run and agent CRUD cycles. Cleanup runs in reverse FK order.
`SERVER_API_ORIGIN` overrides the default trusted extension origin. Connector
OAuth GETs, auth starts, chat execution, MCP config writes and shutdown are
excluded because they have side effects.

A real TS startup was attempted on 2026-10-14 with private temporary state,
server port 19185 and the existing config's CDP port 9005. Startup failed after
three CDP discovery attempts on 127.0.0.1/localhost/::1. No live conformance or
live fixture capture is claimed. A running BrowserOS CDP listener is required
to exercise the optional suite against the complete server.

OpenAPI 3.0 cannot express the current case-insensitive duplicate-header-name
check or header property-name constraints; `LlmHeaders` documents them. Cadence
validation depends on the stored job and is documented on the request rather
than incorrectly rejecting legacy unchanged cadence values in the schema.
