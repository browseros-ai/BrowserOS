# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Coding guidelines

- **Use extensionless imports.** Do not use `.js` extensions in TypeScript imports. Bun resolves `.ts` files automatically.
  ```typescript
  // ✅ Correct
  import { foo } from './utils'
  import type { Bar } from '../types'

  // ❌ Wrong
  import { foo } from './utils.js'
  ```
- Write minimal code comments. Only add comments for non-obvious logic, complex algorithms, or critical warnings. Skip comments for self-explanatory code, obvious function names, and simple operations.
- Logger messages should not include `[prefix]` tags (e.g., `[Config]`, `[HTTP Server]`). Source tracking automatically adds file:line:function in development mode.
- Avoid magic constants scattered in the codebase. Use `@browseros/shared` for all shared configuration:
  - `@browseros/shared/constants/ports` - Port numbers (DEFAULT_PORTS, TEST_PORTS)
  - `@browseros/shared/constants/timeouts` - Timeout values (TIMEOUTS)
  - `@browseros/shared/constants/limits` - Rate limits, pagination, content limits (RATE_LIMITS, AGENT_LIMITS, etc.)
  - `@browseros/shared/constants/urls` - External service URLs (EXTERNAL_URLS)
  - `@browseros/shared/constants/paths` - File system paths (PATHS)
  - `@browseros/shared/types/logger` - Logger interface types (LoggerInterface, LogLevel)

## File Naming Convention

Use **kebab-case** for all file and folder names:

| Type | Convention | Example |
|------|------------|---------|
| Multi-word files | kebab-case | `gemini-agent.ts`, `mcp-context.ts` |
| Single-word files | lowercase | `types.ts`, `browser.ts`, `index.ts` |
| Test files | `.test.ts` suffix | `mcp-context.test.ts` |
| Folders | kebab-case | `rate-limiter/`, `browser-tools/` |

Classes remain PascalCase in code, but live in kebab-case files:
```typescript
// file: gemini-agent.ts
export class GeminiAgent { ... }
```

## Project Overview

**BrowserOS Server** - The automation engine inside BrowserOS. This MCP server powers the built-in AI agent and lets external tools like `claude-code` or `gemini-cli` control the browser. Starts automatically when BrowserOS launches.

## Bun Preferences

Default to using Bun instead of Node.js:

- Use `bun <file>` instead of `node <file>`
- Use `bun test` instead of `jest` or `vitest`
- Use `bun install` instead of `npm install`
- Use `bun run <script>` instead of `npm run <script>`
- Bun automatically loads .env (no dotenv needed)

## Common Commands

```bash
# Start server (development)
bun run start                    # Loads .env.dev automatically

# Testing
bun run test                     # Run tool tests (requires BrowserOS running)
bun run test:tools               # Same as above
bun run test:integration         # Run integration tests
bun run test:sdk                 # Run SDK tests

# Run a single test file
bun --env-file=.env.development test apps/server/tests/path/to/file.test.ts

# Linting
bun run lint                     # Check with Biome
bun run lint:fix                 # Auto-fix with Biome

# Type checking
bun run typecheck                # TypeScript build check

# Build
bun run dev:server               # Build server for development
bun run dev:ext                  # Build extension for development
bun run dist:server              # Build server for production (all targets)
bun run dist:ext                 # Build extension for production

# Refresh models.dev data
bun run generate:models          # Fetches latest from models.dev/api.json
```

## Architecture

This is a monorepo with three packages in `apps/`:

### Server (`apps/server`)
The main MCP server that exposes browser automation tools via HTTP/SSE.

**Entry point:** `apps/server/src/index.ts` → `apps/server/src/main.ts`

**Key components:**
- `src/tools/` - MCP tool definitions, split into:
  - `cdp-based/` - Tools using Chrome DevTools Protocol (navigation, DOM interaction, network, console, emulation, input, etc.)
- `src/common/` - Shared utilities (McpContext, PageCollector, browser connection, identity, db)
- `src/agent/` - AI agent functionality (Gemini adapter, rate limiting, session management)
- `src/http/` - Hono HTTP server with MCP, health, and provider routes

**Tool types:**
- CDP tools require a direct CDP connection (`--cdp-port`)

### Shared (`packages/shared`)
Shared constants, types, and configuration used across packages. Avoids magic numbers.

**Structure:**
- `src/constants/` - Configuration values (ports, timeouts, limits, urls, paths)
- `src/types/` - Shared type definitions (logger)

**Exports:** `@browseros/shared/constants/*`, `@browseros/shared/types/*`

### Communication Flow

```
AI Agent/MCP Client → HTTP Server (Hono) → Tool Handler
                                              ↓
                                   CDP → BrowserOS / Chrome APIs
```

## Creating Packages

When creating new packages in this monorepo:

- **Location:** Packages go in `packages/`, apps go in `apps/`
- **No index.ts:** Don't create or export an `index.ts` - it inflates the bundle with all exports
- **Separate export files:** Keep exports in individual files (e.g., `logger.ts`, `ports.ts`)
- **Import pattern:** `import { X } from "@my-package/name/logger"` - only imports what's needed

**package.json exports:** Must include both `types` and `default` for TypeScript:
```json
"exports": {
  "./constants/ports": {
    "types": "./src/constants/ports.ts",
    "default": "./src/constants/ports.ts"
  },
  "./types/logger": {
    "types": "./src/types/logger.ts",
    "default": "./src/types/logger.ts"
  }
}
```

## Test Organization

Tests are in `apps/server/tests/`:
- `tools/` - Tool tests (require BrowserOS running with CDP), plus ACL scorer tests (standalone)
- `browser/` - Browser backend tests
- `agent/` - Agent tests (compaction, rate limiter)
- `sdk/` - Agent SDK tests
- `__helpers__/` - Test utilities and fixtures

## Self-Testing UI Changes

After making UI changes to the agent extension (`apps/agent/`), you can visually verify them using the CDP inspector script. This connects directly to the browser via Chrome DevTools Protocol and can inspect extension pages (side panel, new tab, etc.) that the agent's own tools cannot see.

### Prerequisites

The dev server must be running:
```bash
bun run dev:watch -- --new
```
Read the output to find the randomized CDP port, then:
```bash
export BROWSEROS_CDP_PORT=<port from output>
```

### Workflow

1. **List all targets** to see what's available:
   ```bash
   bun scripts/dev/inspect-ui.ts targets
   ```

2. **Open the side panel** if it's not already open:
   ```bash
   bun scripts/dev/inspect-ui.ts open-sidepanel
   ```

3. **Take a screenshot** of the side panel:
   ```bash
   bun scripts/dev/inspect-ui.ts screenshot sidepanel /tmp/panel.png
   ```
   Then read `/tmp/panel.png` to view the result.

4. **Get the accessibility tree** for structural verification:
   ```bash
   bun scripts/dev/inspect-ui.ts snapshot sidepanel
   ```

5. **Click an element** by its ID from the snapshot:
   ```bash
   bun scripts/dev/inspect-ui.ts click sidepanel 142
   ```

6. **Fill a text input** by its ID from the snapshot:
   ```bash
   bun scripts/dev/inspect-ui.ts fill sidepanel 85 "search query"
   ```

7. **Evaluate JavaScript** in the extension context:
   ```bash
   bun scripts/dev/inspect-ui.ts eval sidepanel "document.title"
   ```

### Interaction workflow

The typical loop is: snapshot → identify element IDs → click/fill → screenshot to verify.
Element IDs come from the `[number]` in snapshot output (these are `backendDOMNodeId` values).
This uses the same element resolution as the server's MCP tools — no coordinate guessing.

### Target selection

The `<target>` argument can be:
- An **index** from the `targets` output (e.g., `3`)
- A **URL substring** (e.g., `sidepanel`, `newtab`, `chrome-extension://`)
