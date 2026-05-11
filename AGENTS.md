<!-- gitnexus:start -->
# GitNexus — Code Intelligence

This project is indexed by GitNexus as **BrowserOS** (21168 symbols, 43165 relationships, 300 execution flows). Use the GitNexus MCP tools to understand code, assess impact, and navigate safely.

> If any GitNexus tool warns the index is stale, run `npx gitnexus analyze` in terminal first.

## Always Do

- **MUST run impact analysis before editing any symbol.** Before modifying a function, class, or method, run `gitnexus_impact({target: "symbolName", direction: "upstream"})` and report the blast radius (direct callers, affected processes, risk level) to the user.
- **MUST run `gitnexus_detect_changes()` before committing** to verify your changes only affect expected symbols and execution flows.
- **MUST warn the user** if impact analysis returns HIGH or CRITICAL risk before proceeding with edits.
- When exploring unfamiliar code, use `gitnexus_query({query: "concept"})` to find execution flows instead of grepping. It returns process-grouped results ranked by relevance.
- When you need full context on a specific symbol — callers, callees, which execution flows it participates in — use `gitnexus_context({name: "symbolName"})`.

## Never Do

- NEVER edit a function, class, or method without first running `gitnexus_impact` on it.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis.
- NEVER rename symbols with find-and-replace — use `gitnexus_rename` which understands the call graph.
- NEVER commit changes without running `gitnexus_detect_changes()` to check affected scope.

## Resources

| Resource | Use for |
|----------|---------|
| `gitnexus://repo/BrowserOS/context` | Codebase overview, check index freshness |
| `gitnexus://repo/BrowserOS/clusters` | All functional areas |
| `gitnexus://repo/BrowserOS/processes` | All execution flows |
| `gitnexus://repo/BrowserOS/process/{name}` | Step-by-step execution trace |

## CLI

| Task | Read this skill file |
|------|---------------------|
| Understand architecture / "How does X work?" | `.claude/skills/gitnexus/gitnexus-exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.claude/skills/gitnexus/gitnexus-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.claude/skills/gitnexus/gitnexus-debugging/SKILL.md` |
| Rename / extract / split / refactor | `.claude/skills/gitnexus/gitnexus-refactoring/SKILL.md` |
| Tools, resources, schema reference | `.claude/skills/gitnexus/gitnexus-guide/SKILL.md` |
| Index, status, clean, wiki CLI commands | `.claude/skills/gitnexus/gitnexus-cli/SKILL.md` |

<!-- gitnexus:end -->

---

## Dev Deployment (mise / Make)

BrowserOS uses a Go-based CLI (`browseros-dev`) for local dev lifecycle, invoked via `bun run` scripts in `packages/browseros-agent/`.

### Commands

| Script | Command | Purpose |
|--------|---------|--------|
| `dev:setup` | `./tools/dev/run.sh setup` | First-time env setup (deps, profiles) |
| `dev:watch` | `./tools/dev/run.sh watch` | Start agent HMR + server + browser (supervised) |
| `dev:watch:new` | `./tools/dev/run.sh watch --new` | Same but random ports + fresh user-data dir |
| `dev:manual` | `./tools/dev/run.sh watch --manual` | Static agent build instead of WXT HMR |
| `dev:cleanup` | `./tools/dev/run.sh cleanup --target dev` | Kill processes, free ports |
| `dev:reset` | `./tools/dev/run.sh reset --target dev` | Cleanup + delete user-data dir |
| `dev:cleanup:prod` | `./tools/dev/run.sh cleanup --target prod` | Cleanup production instance |
| `dev:reset:dogfood` | `./tools/dev/run.sh reset --target dogfood` | Reset dogfood profile |

### Prerequisites

- **Go** ≥1.21 (`brew install go`)
- **Bun** runtime
- **Lima** (`limactl`) for containerized builds
- AppImage at `BROWSEROS_APP_PATH` (default `~/Downloads/alta/BrowserOS.AppImage`)

### Env Vars

| Variable | Default | Description |
|----------|---------|------------|
| `BROWSEROS_APP_PATH` | `~/Downloads/alta/BrowserOS.AppImage` | Path to BrowserOS AppImage |
| `BROWSEROS_CDP_PORT` | `9000` (prod) / `9005` (dev) | Chrome DevTools Protocol port |
| `BROWSEROS_SERVER_PORT` | `9100` (prod) / `9105` (dev) | Unified server HTTP port |
| `BROWSEROS_EXTENSION_PORT` | `9300` (prod) / `9305` (dev) | Extension port (deprecated, no-op) |

### Desktop Entries

| Entry | Name | Icon | Purpose |
|-------|------|------|--------|
| `browseros.desktop` | BrowserOS | `browseros` | Production AppImage launch |
| `browseros-dev.desktop` | BrowserOS (Dev) | `browseros-dev` (β badge overlay) | Dev instance w/ custom profile + ports |

Installed via `scripts/setup-desktop-entries.sh`. Dev icon gets green **β** badge via PIL overlay.

---

## Config System

Server config loaded in `packages/browseros-agent/apps/server/src/config.ts`.

### Layer Precedence (highest → lowest)

```
CLI flags > Config file (JSON) > Environment variables > Defaults
```

### Key Files

| File | Location | Purpose |
|------|----------|--------|
| `config.sample.json` | `packages/browseros-agent/` | Reference with all keys documented |
| `config.dev.json` | `packages/browseros-agent/` | Dev overrides (offset ports +9005) |
| Custom | any path | Passed via `--config <path>` CLI flag |

### Config Keys

| Key | JSON Path | ENV Var | Default | Description |
|-----|-----------|---------|---------|------------|
| CDP port | `ports.cdp` | `BROWSEROS_CDP_PORT` | `null` | Chrome DevTools Protocol WS port |
| Server port | `ports.server` | `BROWSEROS_SERVER_PORT` | required | Unified HTTP server port |
| Extension port | `ports.extension` | `BROWSEROS_EXTENSION_PORT` | `null` | Deprecated, no-op |
| Resources dir | `directories.resources` | `BROWSEROS_RESOURCES_DIR` | `cwd` | Static assets root |
| Execution dir | `directories.execution` | `BROWSEROS_EXECUTION_DIR` | `cwd` | Logs + runtime output |
| MCP remote | `flags.allow_remote_in_mcp` | — | `false` | Allow non-localhost MCP connections |

### Dev Overrides

`config.dev.json` offsets all ports by +5 from production defaults and skips production-only env var validation (`NODE_ENV !== production`).

---

## Copy Session ID

Utility for copying the active conversation session ID to clipboard.

| Component | File | Role |
|-----------|------|------|
| `copySessionIdToClipboard()` | `.../sidepanel/index/CopySessionId.ts` | Writes `conversationId` to `navigator.clipboard`; returns `false` for empty/null |
| `buildSessionIdLabel()` | same | Short display label: `head...tail` (e.g. `550e...0000`) |
| Copy button | `.../sidepanel/index/ChatHeader.tsx` | `CopyIcon`/`CheckIcon` toggle in header; fires on click |
| Analytics | `@/lib/constants/analyticsEvents.ts` | Event: `sidepanel.session_id.copied` via `track()` |

The button is conditionally rendered when `conversationId` is truthy. On success it shows a ✓ checkmark for 2 seconds.

# Dev Deployment

Use mise tasks as the single entry point for local dev workflow.

| Task | Command | Purpose |
|---|---|---|
| Full restart | `mise run browseros:restart-dev` | Kill → rebuild → start → verify |
| Kill | `mise run browseros:kill-dev` | Kill browser + server processes |
| Build | `mise run browseros:build-dev` | Build dev extension bundle |
| Start | `mise run browseros:start-dev` | Launch AppImage + server + health check |
| Health | `mise run browseros:health-dev` | Check `GET /health` response |

Prerequisites: `mise` (trusted), `bun`, BrowserOS AppImage.

Desktop entries installed by `scripts/setup-desktop-entries.sh`:
- **Prod**: `BrowserOS` (WM_CLASS: `chromium-browser`)
- **Dev**: `BrowserOS (Dev)` with β badge (WM_CLASS: `BrowserOS-Dev`)

### Environment Variables

| Var | Default | Purpose |
|---|---|---|
| `BROWSEROS_APP_PATH` | `$HOME/Downloads/alta/BrowserOS.AppImage` | AppImage location |
| `BROWSEROS_CDP_PORT` | `9010` | Chrome DevTools Protocol port |
| `BROWSEROS_SERVER_PORT` | `9110` | Dev server port |
| `BROWSEROS_EXTENSION_PORT` | `9305` | Extension messaging port |

# Config System

Runtime configuration with bounds validation and layer precedence.

### Layer precedence (highest wins)

1. **ENV vars** — e.g. `BROWSEROS_LIMIT_MAX_TURNS=9999`
2. **File overrides** — `~/.browseros/advanced-config.json` (prod) or `~/.browseros-dev/advanced-config.json` (dev)
3. **Defaults** — in `packages/shared/src/constants/config-schema.ts`

### Key files

| File | Purpose |
|---|---|
| `packages/shared/src/constants/config-schema.ts` | Schema: keys, defaults, bounds, ENV var mapping |
| `packages/shared/src/constants/config-store.ts` | Store: file persistence, ENV override, getter API |
| `packages/shared/src/constants/limits.ts` | Resolved limit values |
| `packages/shared/src/constants/timeouts.ts` | Resolved timeout values |
| `apps/agent/entrypoints/app/advanced-config/` | UI: Advanced Config settings page |
| `apps/server/src/api/routes/config.ts` | API: GET/PUT/DELETE /config |

### Config keys

All limit keys have `min=1`, `max=MAX_SAFE_INTEGER` — no artificial caps.

| Key | Default | ENV |
|---|---|---|
| `AGENT_LIMITS.MAX_TURNS` | 100 | `BROWSEROS_LIMIT_MAX_TURNS` |
| `AGENT_LIMITS.DEFAULT_CONTEXT_WINDOW` | 200000 | `BROWSEROS_LIMIT_DEFAULT_CONTEXT_WINDOW` |
| `TOOL_LIMITS.FILESYSTEM_READ_MAX_CHARS` | 15000 | `BROWSEROS_LIMIT_FILESYSTEM_READ_MAX_CHARS` |
| `TOOL_LIMITS.FILESYSTEM_READ_MAX_LINES` | 500 | `BROWSEROS_LIMIT_FILESYSTEM_READ_MAX_LINES` |
| `TOOL_LIMITS.INLINE_PAGE_CONTENT_MAX_CHARS` | 5000 | `BROWSEROS_LIMIT_INLINE_PAGE_CONTENT_MAX_CHARS` |
| `TIMEOUTS.TOOL_CALL` | 120000 | `BROWSEROS_TIMEOUT_TOOL_CALL` |

Dev overrides in `~/.browseros-dev/advanced-config.json`: max turns=9999, tool timeout=2h, context=20M tokens, chars=1.5M, lines=50K.

**Do NOT modify bounds in config-schema.ts without updating tests.**

# Copy Session ID

ChatHeader has a "Copy Session ID" button (📋 icon) that copies `conversationId` (UUID) to clipboard.
Tracked via `sidepanel.session_id.copied` analytics event.
Implementation: `apps/agent/entrypoints/sidepanel/index/CopySessionId.ts` + `CopySessionId.test.ts`.
