# Advanced Config — Design Spec

## Goal

Expose runtime configuration (timeouts, limits, retention) in a Settings UI page with schema-validated editing. Changes persist to disk and take effect after BrowserOS restart.

## Architecture

```
┌──────────────────┐   GET/PUT    ┌──────────────┐   reads   ┌────────────────────┐
│  Settings Page    │────────────▶ │  Server API  │─────────▶│  advanced-config    │
│  (agent ext)     │◀────────────│  /config      │◀─────────│  .json (disk)      │
└──────────────────┘              └──────┬───────┘           └────────────────────┘
                                         │ reads
                                         ▼
                                  ┌──────────────┐
                                  │ ConfigStore   │  ← replaces frozen const exports
                                  │ (shared pkg)  │
                                  │ ENV > file    │
                                  │ > defaults    │
                                  └──────────────┘
                                         │ serves
                                         ▼
                                  ┌──────────────┐
                                  │ Server agents │
                                  │ compaction    │
                                  │ tool-adapter  │
                                  │ mcp-builder   │
                                  └──────────────┘
```

### ConfigStore Pattern

Replace `export const TIMEOUTS = { TOOL_CALL: envMs(...) }` with a getter-based store:

```ts
// Before: frozen at import
export const TIMEOUTS = { TOOL_CALL: envMs('BROWSEROS_TIMEOUT_TOOL_CALL', 120_000) }

// After: reads merged layer (ENV > file > default)
import { configStore } from './config-store'
export const TIMEOUTS = {
  get TOOL_CALL() { return configStore.get('TIMEOUTS.TOOL_CALL') }
}
```

Consumers don't change — `TIMEOUTS.TOOL_CALL` still works. But the value now reads from a merged layer on each access.

### Persistence

- File: `~/.browseros/advanced-config.json` (separate from `server.json`)
- Format: `{ "TIMEOUTS.TOOL_CALL": 60000, "AGENT_LIMITS.MAX_TURNS": 50 }`
- Only non-default values stored (sparse)

### Restart Flow

- Save → validate → persist to disk → show "Changes saved. Please quit and reopen BrowserOS to apply."
- No live reload. No restart button (browser-level restart needed, not just server).
- `GET /config` returns `active` (current in-memory) vs `pending` (saved to disk, differs from active).

## File Structure

```
packages/shared/src/constants/
  config-store.ts          # NEW: ConfigStore singleton
  config-schema.ts         # NEW: Zod schema for all 32 keys with min/max/unit/risk
  timeouts.ts              # MODIFY: const → getter
  limits.ts                # MODIFY: const → getter
  paths.ts                 # MODIFY: const → getter

apps/server/src/api/routes/
  config.ts                # NEW: GET/PUT /config routes

apps/server/src/api/server.ts  # MODIFY: register config routes

apps/agent/entrypoints/app/
  advanced-config/              # NEW: settings page
    AdvancedConfigPage.tsx
    ConfigGroup.tsx
    ConfigField.tsx
    PendingRestartBanner.tsx
    config-queries.ts

apps/agent/entrypoints/app/App.tsx           # MODIFY: add route
apps/agent/components/sidebar/SettingsSidebar.tsx  # MODIFY: add nav item

apps/server/tests/config.test.ts         # NEW: API + ConfigStore tests
packages/shared/tests/constants/config-store.test.ts  # NEW
```

## Config Schema

All 32 overridable keys, grouped:

### Timeouts (14 keys)

| Key | Default | Unit | Min | Max | Section |
|-----|---------|------|-----|-----|---------|
| TIMEOUTS.TOOL_CALL | 120000 | ms | 1000 | 600000 | safe |
| TIMEOUTS.TOOL_POST_ACTION | 2000 | ms | 0 | 30000 | safe |
| TIMEOUTS.TEST_PROVIDER | 15000 | ms | 1000 | 120000 | safe |
| TIMEOUTS.REFINE_PROMPT | 30000 | ms | 1000 | 120000 | safe |
| TIMEOUTS.MCP_DEFAULT | 5000 | ms | 1000 | 60000 | safe |
| TIMEOUTS.MCP_TRANSPORT_PROBE | 5000 | ms | 1000 | 30000 | safe |
| TIMEOUTS.MCP_CLIENT_CONNECT | 15000 | ms | 1000 | 120000 | safe |
| TIMEOUTS.CDP_CONNECT | 10000 | ms | 1000 | 60000 | safe |
| TIMEOUTS.CDP_CONNECT_RETRY_DELAY | 1000 | ms | 100 | 30000 | dangerous |
| TIMEOUTS.CDP_RECONNECT_DELAY | 5000 | ms | 100 | 60000 | dangerous |
| TIMEOUTS.CDP_KEEPALIVE_INTERVAL | 30000 | ms | 1000 | 300000 | dangerous |
| TIMEOUTS.CDP_KEEPALIVE_TIMEOUT | 10000 | ms | 1000 | 120000 | dangerous |
| TIMEOUTS.CDP_REQUEST_TIMEOUT | 60000 | ms | 1000 | 300000 | safe |
| TIMEOUTS.KLAVIS_FETCH | 30000 | ms | 1000 | 120000 | safe |
| TIMEOUTS.SKILLS_FETCH | 15000 | ms | 1000 | 120000 | safe |
| TIMEOUTS.SKILLS_SYNC_INTERVAL | 2700000 | ms | 60000 | 86400000 | safe |
| TIMEOUTS.NAVIGATION | 10000 | ms | 1000 | 120000 | safe |
| TIMEOUTS.PAGE_LOAD_WAIT | 30000 | ms | 1000 | 120000 | safe |
| TIMEOUTS.PAGE_LOAD_POLL_INTERVAL | 150 | ms | 10 | 5000 | dangerous |
| TIMEOUTS.STABLE_DOM | 3000 | ms | 100 | 30000 | safe |
| TIMEOUTS.FILE_CHOOSER | 3000 | ms | 100 | 30000 | safe |
| TIMEOUTS.OAUTH_FLOW_TTL | 300000 | ms | 10000 | 3600000 | safe |
| TIMEOUTS.OAUTH_TOKEN_EXPIRY_BUFFER | 300000 | ms | 10000 | 3600000 | dangerous |
| TIMEOUTS.OAUTH_POLL_INTERVAL | 2000 | ms | 100 | 30000 | dangerous |
| TIMEOUTS.OAUTH_POLL_TIMEOUT | 300000 | ms | 10000 | 3600000 | safe |
| TIMEOUTS.DEVICE_CODE_POLL_SAFETY_MARGIN | 3000 | ms | 100 | 30000 | dangerous |

### Limits (8 keys)

| Key | Default | Unit | Min | Max | Section |
|-----|---------|------|-----|-----|---------|
| AGENT_LIMITS.MAX_TURNS | 100 | turns | 1 | 1000 | safe |
| AGENT_LIMITS.DEFAULT_CONTEXT_WINDOW | 200000 | tokens | 1000 | 2000000 | safe |
| AGENT_LIMITS.COMPACTION_SUMMARIZATION_TIMEOUT_MS | 60000 | ms | 5000 | 600000 | dangerous |
| AGENT_LIMITS.COMPACTION_MAX_SUMMARIZATION_INPUT | 100000 | tokens | 1000 | 1000000 | dangerous |
| AGENT_LIMITS.COMPACTION_TOOL_OUTPUT_MAX_CHARS | 15000 | chars | 100 | 1000000 | dangerous |
| TOOL_LIMITS.INLINE_PAGE_CONTENT_MAX_CHARS | 5000 | chars | 100 | 100000 | safe |
| TOOL_LIMITS.FILESYSTEM_READ_MAX_LINES | 500 | lines | 1 | 10000 | safe |
| TOOL_LIMITS.FILESYSTEM_READ_MAX_CHARS | 15000 | chars | 100 | 100000 | safe |

### Retention (3 keys)

| Key | Default | Unit | Min | Max | Section |
|-----|---------|------|-----|-----|---------|
| PATHS.SOUL_MAX_LINES | 150 | lines | 10 | 10000 | safe |
| PATHS.MEMORY_RETENTION_DAYS | 30 | days | 1 | 365 | safe |
| PATHS.SESSION_RETENTION_DAYS | 30 | days | 1 | 365 | safe |

### Dangerous Keys — Risk Explanations

Each dangerous key shows its risk explanation in the UI:

- **CDP_CONNECT_RETRY_DELAY**: "Too low → retry storms. Too high → slow recovery from disconnects."
- **CDP_RECONNECT_DELAY**: "Controls delay before reconnecting to the browser. Too low → rapid retry loop."
- **CDP_KEEPALIVE_INTERVAL**: "How often to ping the browser. Too low → unnecessary load. Too high → missed disconnects."
- **CDP_KEEPALIVE_TIMEOUT**: "How long to wait for keepalive response. Too low → false disconnects."
- **PAGE_LOAD_POLL_INTERVAL**: "Polling frequency for page load detection. Too low → CPU waste. Too high → slow detection."
- **OAUTH_TOKEN_EXPIRY_BUFFER**: "How far before expiry to refresh tokens. Too low → token expires mid-flow."
- **OAUTH_POLL_INTERVAL**: "Polling frequency during OAuth flows. Too low → API rate limits."
- **DEVICE_CODE_POLL_SAFETY_MARGIN**: "Safety margin before device code expiry. Too low → flow fails prematurely."
- **COMPACTION_SUMMARIZATION_TIMEOUT_MS**: "Timeout for LLM summarization during compaction. Too low → incomplete summaries."
- **COMPACTION_MAX_SUMMARIZABLE_INPUT**: "Max input tokens for summarization. Too low → context loss. Too high → API costs."
- **COMPACTION_TOOL_OUTPUT_MAX_CHARS**: "Max chars kept from tool outputs in transcript. Too low → lost context. Too high → memory bloat."

## UI Design

### Page Layout

```
┌──────────────────────────────────────────────────────────┐
│  Advanced Config                                    [?]   │
├──────────────────────────────────────────────────────────┤
│  ⚠ Changes require BrowserOS restart to take effect.     │
│  2 pending changes saved. Quit & reopen to apply.        │
├──────────────────────────────────────────────────────────┤
│                                                          │
│  ── Timeouts ────────────────────────────────────────    │
│  Tool Call Timeout    [120000    ] ms   (default: 120000) │
│  MCP Client Connect   [15000     ] ms   (default: 15000)  │
│  ...                                                     │
│                                                          │
│  ── Limits ──────────────────────────────────────────    │
│  Max Turns            [100       ] turns                  │
│  Context Window       [200000    ] tokens                 │
│  ...                                                     │
│                                                          │
│  ── Retention ───────────────────────────────────────    │
│  Soul Max Lines       [150       ] lines                  │
│  Memory Retention     [30        ] days                   │
│  Session Retention    [30        ] days                   │
│                                                          │
│  ── ⚠ Dangerous ────────────────────────────────────    │
│  ⚠ CDP Connect Retry Delay                         [1000]│
│    "Too low → retry storms. Too high → slow recovery."   │
│  ⚠ Compaction Summarization Timeout                [60s] │
│    "Too low → incomplete summaries during compaction."    │
│  ...                                                     │
│                                                          │
│  [Reset to Defaults]              [Save Changes]         │
└──────────────────────────────────────────────────────────┘
```

### Behaviors

- **Validation**: Per-field inline validation on blur. Red border + error message if outside min/max or non-integer.
- **Save button**: Disabled until all fields pass validation. On save → persist to disk → show "saved" toast.
- **Reset**: Per-field reset (revert to default) + "Reset All to Defaults" button.
- **Pending restart banner**: Shown at top when saved values differ from active values. Text: "Changes saved. Please quit and reopen BrowserOS to apply."
- **Dangerous section**: Collapsed by default. Yellow/amber accent. Each field shows risk explanation.

## Server API

### `GET /config`

```json
{
  "active": { "TIMEOUTS.TOOL_CALL": 120000, ... },
  "pending": { "TIMEOUTS.TOOL_CALL": 60000 },
  "defaults": { "TIMEOUTS.TOOL_CALL": 120000, ... },
  "schema": { "TIMEOUTS.TOOL_CALL": { "min": 1000, "max": 600000, "unit": "ms", "section": "safe", "group": "Timeouts", "label": "Tool Call Timeout", "description": "..." } },
  "hasPendingChanges": true
}
```

### `PUT /config`

```json
// Request
{ "overrides": { "TIMEOUTS.TOOL_CALL": 60000, "AGENT_LIMITS.MAX_TURNS": 50 } }

// Response 200
{ "ok": true, "saved": 2, "hasPendingChanges": true }

// Response 400 (validation failure)
{ "ok": false, "errors": [{ "key": "TIMEOUTS.TOOL_CALL", "message": "Value 50 is below minimum 1000" }] }
```

### `DELETE /config`

Resets all overrides (deletes file). Returns defaults.

## Testing

- ConfigStore: merge layer tests (ENV > file > default), invalid file handling, sparse overrides
- Schema: boundary validation (min-1, max+1, non-integer, negative, zero)
- API: GET returns correct merge, PUT validates before save, DELETE resets
- UI: field validation, save disabled on invalid, pending banner visibility
