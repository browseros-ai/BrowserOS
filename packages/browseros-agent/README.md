# BrowserOS Agent

The agent platform powering [BrowserOS](https://github.com/browseros-ai/BrowserOS) — contains the MCP server, agent UI, CLI, evaluation framework, and SDK.

## Monorepo Structure

```
apps/
  server/          # Bun server - MCP endpoints + agent loop
  agent/           # Agent UI (Chrome extension)
  cli/             # Go CLI for controlling BrowserOS from the terminal
  eval/            # Evaluation framework for benchmarking agents

packages/
  agent-sdk/       # Node.js SDK (@browseros-ai/agent-sdk)
  cdp-protocol/    # Type-safe Chrome DevTools Protocol bindings
  shared/          # Shared constants (ports, timeouts, limits)
```

| Package | Description |
|---------|-------------|
| `apps/server` | Bun server exposing MCP tools and running the agent loop |
| `apps/agent` | Agent UI — Chrome extension for the chat interface |
| `apps/cli` | Go CLI — control BrowserOS from the terminal or AI coding agents |
| `apps/eval` | Benchmark framework — WebVoyager, Mind2Web evaluation |
| `packages/agent-sdk` | Node.js SDK for browser automation with natural language |
| `packages/cdp-protocol` | Auto-generated CDP type bindings used by the server |
| `packages/shared` | Shared constants used across packages |

## Architecture

- `apps/server`: Bun server which contains the agent loop and tools.
- `apps/agent`: Agent UI (Chrome extension).

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
│   /health ─── Health check                                               │
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

| Port | Env Variable | Purpose |
|------|--------------|---------|
| 9100 | `BROWSEROS_SERVER_PORT` | HTTP server - MCP endpoints, agent chat, health |
| 9000 | `BROWSEROS_CDP_PORT` | Chromium CDP server (BrowserOS Server connects as client) |
| 9300 | `BROWSEROS_EXTENSION_PORT` | Legacy BrowserOS launch arg kept for compatibility; not used by the server |

## Development

### Setup

Requires [process-compose](https://github.com/F1bonacc1/process-compose):

```bash
brew install process-compose
```

```bash
# Copy environment files for each package
cp apps/server/.env.example apps/server/.env.development
cp apps/agent/.env.example apps/agent/.env.development
cp apps/server/.env.production.example apps/server/.env.production

# Start the full dev environment
process-compose up
```

The `process-compose up` command runs the following in order:
1. `bun install` — installs dependencies
2. `bun --cwd apps/agent codegen` — generates agent code
3. `bun --cwd apps/server start` and `bun --cwd apps/agent dev` — starts server and agent in parallel

### Environment Variables

Runtime uses `.env.development`, while production artifact builds use `.env.production`:

- `apps/server/.env.development` - Server runtime configuration for local dev
- `apps/server/.env.production` - Server production artifact build configuration
- `apps/agent/.env.development` - Agent UI configuration

**Server Variables** (`apps/server/.env.development`)

| Variable | Default | Description |
|----------|---------|-------------|
| `BROWSEROS_SERVER_PORT` | 9100 | HTTP server port (MCP, chat, health) |
| `BROWSEROS_CDP_PORT` | 9000 | Chromium CDP port (server connects as client) |
| `BROWSEROS_EXTENSION_PORT` | 9300 | Legacy BrowserOS launch arg kept for compatibility |
| `BROWSEROS_CONFIG_URL` | - | Remote config endpoint for rate limits |
| `BROWSEROS_INSTALL_ID` | - | Unique installation identifier (analytics) |
| `BROWSEROS_CLIENT_ID` | - | Client identifier (analytics) |
| `POSTHOG_API_KEY` | - | Server-side PostHog API key |
| `SENTRY_DSN` | - | Server-side Sentry DSN |
| `BROWSEROS_TEST_HEADLESS` | false | Headless mode for server tests |

**Server Production Build Variables** (`apps/server/.env.production`)

Copy from `apps/server/.env.production.example` before running `build:server`.
`build:server` requires all values below except `R2_DOWNLOAD_PREFIX` and `R2_UPLOAD_PREFIX`.

| Variable | Default | Description |
|----------|---------|-------------|
| `BROWSEROS_CONFIG_URL` | - | Remote config endpoint baked into prod binary |
| `CODEGEN_SERVICE_URL` | - | Graph/codegen backend URL baked into prod binary |
| `POSTHOG_API_KEY` | - | PostHog key baked into prod binary |
| `SENTRY_DSN` | - | Sentry DSN baked into prod binary |
| `R2_ACCOUNT_ID` | - | Cloudflare account id for production artifact downloads/uploads |
| `R2_ACCESS_KEY_ID` | - | Cloudflare R2 access key id |
| `R2_SECRET_ACCESS_KEY` | - | Cloudflare R2 secret access key |
| `R2_BUCKET` | - | Cloudflare R2 bucket name |
| `R2_DOWNLOAD_PREFIX` | - | Optional prefix prepended to third-party resource object keys |
| `R2_UPLOAD_PREFIX` | `server/prod-resources` | Optional prefix for uploaded artifact zips |

**Agent Variables** (`apps/agent/.env.development`)

| Variable | Default | Description |
|----------|---------|-------------|
| `BROWSEROS_SERVER_PORT` | 9100 | Passed to BrowserOS via CLI args |
| `BROWSEROS_CDP_PORT` | 9000 | Passed to BrowserOS via CLI args |
| `BROWSEROS_EXTENSION_PORT` | 9300 | Legacy BrowserOS CLI arg still passed for compatibility |
| `VITE_BROWSEROS_SERVER_PORT` | 9100 | Agent UI connects to server (must match `BROWSEROS_SERVER_PORT`) |
| `BROWSEROS_BINARY` | - | Path to BrowserOS binary |
| `USE_BROWSEROS_BINARY` | true | Use BrowserOS instead of default Chrome |
| `VITE_PUBLIC_POSTHOG_KEY` | - | Agent UI PostHog key |
| `VITE_PUBLIC_SENTRY_DSN` | - | Agent UI Sentry DSN |

> **Note:** Port variables are duplicated in both files and must be kept in sync when running server and agent together.

### Commands

```bash
# Start
bun run start:server          # Start the server
bun run start:agent           # Start agent extension (dev mode)

# Build
bun run build                 # Build server and agent
bun run build:server          # Build production server resource artifacts and upload zips to R2
bun run build:agent           # Build agent extension

# Test
bun run test                  # Run standard tests
bun run test:cdp              # Run CDP-based tests
bun run test:integration      # Run integration tests

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
