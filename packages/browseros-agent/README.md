# BrowserOS Agent

The agent platform powering [BrowserOS](https://github.com/browseros-ai/BrowserOS) — contains the MCP server, agent UI, CLI, and shared packages.

## Monorepo Structure

```
apps/
  server/          # Bun server - MCP endpoints + agent loop
  app/             # BrowserOS app UI (Chrome extension)
  cli/             # Go CLI for controlling BrowserOS from the terminal
packages/
  cdp-protocol/    # Type-safe Chrome DevTools Protocol bindings
  shared/          # Shared constants (ports, timeouts, limits)
```

| Package | Description |
|---------|-------------|
| `apps/server` | Bun server exposing MCP tools and running the agent loop |
| `apps/app` | BrowserOS app UI — Chrome extension for the chat interface |
| `apps/cli` | Go CLI — control BrowserOS from the terminal or AI coding agents |
| `packages/cdp-protocol` | Auto-generated CDP type bindings used by the server |
| `packages/shared` | Shared constants used across packages |

## Architecture

- `apps/server`: Bun server which contains the agent loop and tools.
- `apps/app`: BrowserOS app UI (Chrome extension).

```
┌──────────────────────────────────────────────────────────────────────────┐
│                         MCP Clients                                  │
│                (Agent UI, claude-code via MCP)                           │
└──────────────────────────────────────────────────────────────────────────┘
                                    │
                                    │ HTTP/SSE
                                    ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                 BrowserOS Server (serverPort: 9100)                      │
│                                                                          │
│   /mcp ─────── MCP tool endpoints                                        │
│   /chat ────── Agent streaming                                           │
│   /system/health ─ Health check                                          │
│                                                                          │
│   Tools:                                                                 │
│   └── CDP-backed browser tools (tabs, navigation, input, screenshots,   │
│       bookmarks, history, console, DOM, tab groups, windows, ...)       │
└──────────────────────────────────────────────────────────────────────────┘
                                    │
                                    │ CDP (client)
                                    ▼
                         ┌─────────────────────┐
                         │   Chromium CDP      │
                         │  (cdpPort: 9000)    │
                         │                     │
                         │ Server connects     │
                         │ TO this as client   │
                         └─────────────────────┘
```

### Ports

| Port | Sidecar Field | Purpose |
|------|---------------|---------|
| 9100 | `ports.server` | HTTP server - MCP endpoints, agent chat, health |
| 9000 | `ports.cdp` | Chromium CDP server (BrowserOS Server connects as client) |
| 9100 | `ports.proxy` | Browser proxy port emitted by Chromium for managed sidecars |

## Development

### Setup

```bash
# Copy the root development environment file
cp .env.development.example .env.development

# Install deps and generate agent code
bun run dev:setup

# Start the full dev environment
bun run dev:watch
```

`dev:watch` starts the server and app UI immediately.
Existing checkouts with old per-app env files should run `bun run env:migrate` to merge those values into the root files.
For release builds, copy `.env.production.example` to `.env.production` and fill the production-only secrets before running build or upload scripts.

### Environment Variables

The monorepo has two root env files:

- `.env.development` - local development, tests, app/server runs, and codegen inputs.
- `.env.production` - release builds and upload scripts.

Both are gitignored. Their tracked templates, `.env.development.example` and `.env.production.example`, are generated from `@browseros/shared/env/registry`; run `bun run env:examples` after changing the registry. CI drift-checks the generated examples.

Fresh clone setup is: copy `.env.development.example` to `.env.development`, fill any secrets needed for the workflow, then run `bun run dev:*`. Existing checkouts can run `bun run env:migrate` to merge old per-app values into the root files.

**Server Sidecar Config** (`--config <path>`)

The server and Claw server read startup ports, resource directories, execution directories, and instance metadata from a sidecar JSON file. `tools/dev`, dogfood, and Chromium-managed sidecar launches generate this file and pass it with `--config`.

| Field | Description |
|-------|-------------|
| `ports.server` | HTTP server port (MCP, chat, health) |
| `ports.cdp` | Chromium CDP port (server connects as client) |
| `ports.proxy` | Browser proxy port emitted by Chromium |
| `directories.resources` | Packaged resources root |
| `directories.execution` | Runtime execution/log/config directory |
| `instance.*` | Optional browser/client metadata |

**Root Env Sections**

| Section | File | Purpose |
|---------|------|---------|
| `dev-tools` | `.env.development` | Optional codegen and local tooling inputs such as `CDP_PROTOCOL_JSON` and `BROWSEROS_BINARY`. |
| `app` | `.env.development` | Browser extension and local BrowserOS launch settings, including dev ports, public Vite values, source-map upload settings, and optional GraphQL schema path. |
| `claw` | `.env.development` | Optional Claw app/server overrides such as Claw API URL, user-data dir, CDP port, and `BROWSERCLAW_DIR`. |
| `server` | `.env.development`, `.env.production` | Server config URL, telemetry, Sentry, `NODE_ENV`, log level, and local server test settings. |
| `build` | `.env.production` | Build-time production values such as `AGENT_RUNNER_JWT_SECRET`. |
| `upload` | `.env.production` | Cloudflare R2 credentials and bucket for production artifact uploads. |

Production build and upload scripts read root `.env.production` plus exported process env through the shared loader in `@browseros/shared/env/*`; exported process env takes precedence. Missing required values fail with an error naming the key, section, and root file.

### Commands

```bash
# Start
bun run dev:watch             # Start server and app with generated sidecar config
bun run start:server          # Start the server from the repo root
cd apps/server && bun --env-file=../../.env.development src/index.ts --config ../../config.dev.json

# Build
bun run build                 # Build server and agent
bun run build:server          # Build production server resource artifacts and upload zips to R2
bun run build:agent           # Build agent extension

# Test
bun run test                  # Run all tests
bun run test:all              # Run all tests
bun run test:main             # Run key server tools and integration tests

# App-specific test groups (from packages/browseros-agent)
cd apps/server && bun run test:tools
cd apps/server && bun run test:cdp
cd apps/server && bun run test:integration

# Quality
bun run lint                  # Check with Biome
bun run lint:fix              # Auto-fix
bun run typecheck             # TypeScript check
```

`build:server` now emits artifacts under `dist/prod/server/<target>/` and zip files under `dist/prod/server/`.

Direct server build script options:

```bash
bun scripts/build/server.ts --target=all
bun scripts/build/server.ts --target=darwin-arm64,linux-x64
bun scripts/build/server.ts --target=all --manifest=scripts/build/config/server-prod-resources.json
bun scripts/build/server.ts --target=all --no-upload
```

## License

AGPL-3.0
