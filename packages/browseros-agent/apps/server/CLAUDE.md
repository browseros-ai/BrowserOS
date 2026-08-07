# BrowserOS Server contributor ground rules

The Bun server is a Hono HTTP app that exposes MCP tools, drives BrowserOS through CDP, and runs the AI SDK agent loop.

## Before you push

From the monorepo root:

```
bun run lint
bun run typecheck
bun run test:main
```

For focused server work:

```
cd apps/server && bun run test:agent
cd apps/server && bun run test:api
cd apps/server && bun run test:browser
cd apps/server && bun run test:tools
```

Tool, browser, and integration tests may require a running BrowserOS/CDP target. For local server artifact checks, prefer `bun run build:server:test`; use `bun scripts/build/server.ts --target=all --no-upload` when you need all targets without R2 upload.

## Entry points

- `src/index.ts` loads polyfills/config, creates `Application`, starts it, and maps startup failures to exit codes.
- `src/main.ts` owns lifecycle: runtime setup, DB/identity/metrics init, CDP connection, browser wrapper, tool registry, HTTP startup, and shutdown.
- `src/api/server.ts` composes Hono routes for `/health`, `/status`, `/chat`, `/mcp`, `/klavis`, `/agents`, provider testing, prompt refinement, and shutdown.

## Project shape

```
apps/server/
|- src/
|  |- api/          Hono routes, middleware, route services, HTTP utilities
|  |- agent/        AI SDK agent loop, providers, sessions, MCP client setup
|  |- browser/      Browser facade plus CDP-backed core connection/snapshot/input
|  |- lib/          DB, identity, metrics, OAuth, runtime, clients, process helpers
|  |- tools/        MCP tool registry and browser/filesystem/tool implementations
|  |- index.ts      process entry
|  `- main.ts       application lifecycle
`- tests/           grouped by agent, api, browser, lib, tools
```

## Server conventions

- HTTP routes use Hono. Add routes in `src/api/routes/` and compose them in `src/api/server.ts`.
- MCP tool registration flows through `src/tools/registry.ts` and the tool implementation folders. Keep tool names, labels, schemas, and responses in sync.
- CDP-backed browser behavior lives under `src/browser/` and `src/browser/core/`; tools should use that layer instead of speaking raw CDP when a local abstraction exists.
- Agent behavior lives under `src/agent/` and uses the AI SDK provider/tool loop. External MCP clients are per-session and built through `mcp-builder.ts`.
- CDP and server ports are required in the sidecar JSON passed through `--config`.
- Tests live under `apps/server/tests/`; use the closest group runner before broad suites.

## Release gate

Server production resources must not package VM-only Lima resources. Keep `scripts/build/server/stage.test.ts` green: `scripts/build/config/server-prod-resources.json` should exclude `third_party/lima` and `resources/vm/` while still packaging Bun and DB migrations.
