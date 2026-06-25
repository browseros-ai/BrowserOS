# BrowserOS Server

MCP server and AI agent loop powering BrowserOS browser automation. This is the core backend — it connects to Chromium via CDP, exposes 53+ MCP tools, and runs the AI agent that interprets natural language into browser actions.

> **Runtime:** [Bun](https://bun.sh) · **Framework:** [Hono](https://hono.dev) · **AI:** [Vercel AI SDK](https://sdk.vercel.ai) · **License:** [AGPL-3.0](../../../../LICENSE)

## Architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│                         MCP Clients                                  │
│           (Agent UI, Claude Code, Gemini CLI, browseros-cli)         │
└──────────────────────────────────────────────────────────────────────┘
                                │
                                │ HTTP / SSE / StreamableHTTP
                                ▼
┌──────────────────────────────────────────────────────────────────────┐
│                    BrowserOS Server (Bun)                             │
│                                                                      │
│   /mcp ─────── MCP tool endpoints (53+ tools)                       │
│   /chat ────── Agent streaming (AI SDK)                              │
│   /health ─── Health check                                           │
│                                                                      │
│   ┌─────────────────────────────────────────────────────────────┐   │
│   │  Agent Loop                                                  │   │
│   │  ├── Multi-provider AI SDK (OpenAI, Anthropic, Google, ...) │   │
│   │  ├── Session & conversation management                       │   │
│   │  ├── Context overflow handling + compaction                  │   │
│   │  └── MCP client for external tool servers                    │   │
│   └─────────────────────────────────────────────────────────────┘   │
│                                                                      │
│   ┌─────────────────────────────────────────────────────────────┐   │
│   │  CDP-backed browser tools                                   │   │
│   │  (tabs, bookmarks, history, navigation, tab groups,         │   │
│   │   screenshots, DOM, network, console, input)                │   │
│   └─────────────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────────────┘
                                │
                                │ Chrome DevTools Protocol
                                ▼
                     ┌─────────────────────┐
                     │   Chromium CDP      │
                     │  (port 9000)        │
                     │                     │
                     │  DOM, network,      │
                     │  input, screenshots │
                     └─────────────────────┘
```

## MCP Tools

Tools organized by category:

| Category | Tools |
|----------|-------|
| **Navigation** | `new_page`, `navigate`, `go_back`, `go_forward`, `reload` |
| **Input** | `click`, `type`, `press_key`, `hover`, `scroll`, `drag`, `fill`, `clear`, `focus`, `check`, `uncheck`, `select_option`, `upload_file` |
| **Observation** | `take_snapshot`, `take_enhanced_snapshot`, `extract_text`, `extract_links` |
| **Screenshots** | `take_screenshot`, `save_screenshot` |
| **Evaluation** | `evaluate_script` |
| **Pages** | `list_pages`, `active_page`, `close_page`, `new_hidden_page` |
| **Windows** | `window_list`, `window_create`, `window_close`, `window_activate` |
| **Bookmarks** | `bookmark_list`, `bookmark_create`, `bookmark_remove`, `bookmark_update`, `bookmark_move`, `bookmark_search` |
| **History** | `history_search`, `history_recent`, `history_delete`, `history_delete_range` |
| **Tab Groups** | `group_list`, `group_create`, `group_update`, `group_ungroup`, `group_close` |
| **Filesystem** | `ls`, `read`, `write`, `edit`, `find`, `grep`, `bash` |
| **DOM** | `dom`, `dom_search` |
| **Console** | `get_console_messages` |
| **Other** | `browseros_info`, `handle_dialog`, `wait_for`, `download`, `export_pdf`, `output_file`, `nudges` |

## Agent Loop

The agent loop uses the [Vercel AI SDK](https://sdk.vercel.ai) to orchestrate multi-step browser automation:

- **Multi-provider support** — OpenAI, Anthropic, Google, Azure, Bedrock, OpenRouter, Ollama, LM Studio, and any OpenAI-compatible endpoint
- **Session management** — conversations persist in a local SQLite database
- **Context overflow handling** — automatic message compaction when context windows fill up
- **MCP client** — connects to external MCP servers for additional tool access (40+ app integrations)
- **Tool adapter** — bridges MCP tool definitions to AI SDK tool format

### Provider Factory

The provider factory (`src/agent/provider-factory.ts`) creates AI SDK providers from runtime configuration, supporting hot-swapping between providers without restart.

## Directory Structure

```
apps/server/
├── src/
│   ├── index.ts               # Server entry point
│   ├── main.ts                # Server initialization
│   ├── api/                   # HTTP route handlers
│   ├── agent/                 # Agent loop
│   │   ├── ai-sdk-agent.ts    # Main agent implementation
│   │   ├── provider-factory.ts# LLM provider factory
│   │   ├── session-store.ts   # Conversation persistence
│   │   ├── compaction.ts      # Context window management
│   │   ├── mcp-builder.ts     # External MCP client setup
│   │   └── tool-adapter.ts    # MCP → AI SDK tool bridge
│   ├── browser/               # Browser connection layer
│   ├── tools/                 # MCP tool implementations
│   │   ├── navigation.ts
│   │   ├── input.ts
│   │   ├── snapshot.ts
│   │   ├── filesystem/
│   │   └── ...
│   ├── lib/                   # Shared utilities
│   └── rpc.ts                 # JSON-RPC type definitions
├── tests/
│   ├── tools/                 # Tool-level tests
│   └── server.integration.test.ts
└── package.json
```

## Development

### Prerequisites

- [Bun](https://bun.sh) runtime
- A running BrowserOS instance (for CDP connectivity)

### Setup

```bash
# Copy environment files
cp .env.example .env.development

# Start the server (with hot reload)
bun run start
```

See the [agent monorepo README](../../README.md) for full environment variable reference and `dev:watch` setup.

### Testing

```bash
bun run test:tools          # Tool-level tests
bun run test:integration    # Full integration tests (requires running BrowserOS)
```

### Building

```bash
# Build cross-platform server binaries
bun run build

# Build for specific targets
bun scripts/build/server.ts --target=darwin-arm64,linux-x64

# Build without uploading to R2
bun scripts/build/server.ts --target=all --no-upload
```

## Release Flow

Server releases use annotated component tags. First bump `packages/browseros-agent/apps/server/package.json` in a PR, merge that version commit to the default branch, then tag the merged commit:

```bash
git tag -a agent-server/v0.0.122 -m "agent-server v0.0.122"
git push origin agent-server/v0.0.122
```

The release workflow validates that the tag version matches `apps/server/package.json`, that the tagged commit is reachable from the default branch, and that the version is newer than existing `browseros-server-v*` and `agent-server/v*` tags. Legacy `browseros-server-vX.Y.Z` tags remain historical; new GitHub Releases use `agent-server/vX.Y.Z`.

The workflow-call and nightly paths can still build/upload server artifacts without publishing a GitHub Release by setting `publish_github_release=false`; that preserves the existing `bump_server_version.py` flow for target-specific builds.

## Ports

| Port | Env Variable | Purpose |
|------|-------------|---------|
| 9100 | `BROWSEROS_SERVER_PORT` | HTTP server (MCP, chat, health) |
| 9000 | `BROWSEROS_CDP_PORT` | Chromium CDP (server connects as client) |
| 9300 | `BROWSEROS_EXTENSION_PORT` | Legacy BrowserOS launch arg kept for compatibility |
