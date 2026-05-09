# REQ: Expose Hardcoded Constants to Environment / Config File

## Status: TODO

## Problem

Three constant files contain **all-hardcoded values** with zero runtime override capability:

- `packages/browseros-agent/packages/shared/src/constants/timeouts.ts` — 20+ timeout values
- `packages/browseros-agent/packages/shared/src/constants/limits.ts` — limits and thresholds
- `packages/browseros-agent/packages/shared/src/constants/paths.ts` — retention and sizing

Additionally, `tool-adapter.ts` hardcodes `120_000ms` instead of referencing `TIMEOUTS.TOOL_CALL`.

This forces code changes to tune any of these values, preventing operators from adjusting behavior for different environments (local dev, CI, production, tailscale remote, etc.).

## Scope

### In Scope

Make the following values configurable via environment variables with current values as defaults:

#### timeouts.ts

| Constant | Current (ms) | ENV Override |
|---|---|---|
| `TOOL_CALL` | `120_000` | `BROWSEROS_TIMEOUT_TOOL_CALL` |
| `TOOL_POST_ACTION` | `2_000` | `BROWSEROS_TIMEOUT_TOOL_POST_ACTION` |
| `MCP_DEFAULT` | `5_000` | `BROWSEROS_TIMEOUT_MCP_DEFAULT` |
| `MCP_TRANSPORT_PROBE` | `5_000` | `BROWSEROS_TIMEOUT_MCP_TRANSPORT_PROBE` |
| `MCP_CLIENT_CONNECT` | `15_000` | `BROWSEROS_TIMEOUT_MCP_CLIENT_CONNECT` |
| `CDP_CONNECT` | `10_000` | `BROWSEROS_TIMEOUT_CDP_CONNECT` |
| `CDP_CONNECT_RETRY_DELAY` | `1_000` | `BROWSEROS_TIMEOUT_CDP_CONNECT_RETRY_DELAY` |
| `CDP_RECONNECT_DELAY` | `5_000` | `BROWSEROS_TIMEOUT_CDP_RECONNECT_DELAY` |
| `CDP_KEEPALIVE_INTERVAL` | `30_000` | `BROWSEROS_TIMEOUT_CDP_KEEPALIVE_INTERVAL` |
| `CDP_KEEPALIVE_TIMEOUT` | `10_000` | `BROWSEROS_TIMEOUT_CDP_KEEPALIVE_TIMEOUT` |
| `CDP_REQUEST_TIMEOUT` | `60_000` | `BROWSEROS_TIMEOUT_CDP_REQUEST_TIMEOUT` |
| `KLAVIS_FETCH` | `30_000` | `BROWSEROS_TIMEOUT_KLAVIS_FETCH` |
| `SKILLS_FETCH` | `15_000` | `BROWSEROS_TIMEOUT_SKILLS_FETCH` |
| `SKILLS_SYNC_INTERVAL` | `2_700_000` | `BROWSEROS_TIMEOUT_SKILLS_SYNC_INTERVAL` |
| `NAVIGATION` | `10_000` | `BROWSEROS_TIMEOUT_NAVIGATION` |
| `PAGE_LOAD_WAIT` | `30_000` | `BROWSEROS_TIMEOUT_PAGE_LOAD_WAIT` |
| `PAGE_LOAD_POLL_INTERVAL` | `150` | `BROWSEROS_TIMEOUT_PAGE_LOAD_POLL_INTERVAL` |
| `STABLE_DOM` | `3_000` | `BROWSEROS_TIMEOUT_STABLE_DOM` |
| `FILE_CHOOSER` | `3_000` | `BROWSEROS_TIMEOUT_FILE_CHOOSER` |
| `OAUTH_FLOW_TTL` | `300_000` | `BROWSEROS_TIMEOUT_OAUTH_FLOW_TTL` |
| `OAUTH_TOKEN_EXPIRY_BUFFER` | `300_000` | `BROWSEROS_TIMEOUT_OAUTH_TOKEN_EXPIRY_BUFFER` |
| `OAUTH_POLL_INTERVAL` | `2_000` | `BROWSEROS_TIMEOUT_OAUTH_POLL_INTERVAL` |
| `OAUTH_POLL_TIMEOUT` | `300_000` | `BROWSEROS_TIMEOUT_OAUTH_POLL_TIMEOUT` |
| `DEVICE_CODE_POLL_SAFETY_MARGIN` | `3_000` | `BROWSEROS_TIMEOUT_DEVICE_CODE_POLL_SAFETY_MARGIN` |

#### limits.ts

| Constant | Current | ENV Override |
|---|---|---|
| `AGENT_LIMITS.MAX_TURNS` | `100` | `BROWSEROS_LIMIT_MAX_TURNS` |
| `AGENT_LIMITS.DEFAULT_CONTEXT_WINDOW` | `200_000` | `BROWSEROS_LIMIT_DEFAULT_CONTEXT_WINDOW` |
| `AGENT_LIMITS.COMPACTION_SUMMARIZATION_TIMEOUT_MS` | `60_000` | `BROWSEROS_TIMEOUT_COMPACTION_SUMMARIZATION` |
| `TOOL_LIMITS.FILESYSTEM_READ_MAX_LINES` | `500` | `BROWSEROS_LIMIT_FILESYSTEM_READ_MAX_LINES` |
| `TOOL_LIMITS.FILESYSTEM_READ_MAX_CHARS` | `15_000` | `BROWSEROS_LIMIT_FILESYSTEM_READ_MAX_CHARS` |
| `TOOL_LIMITS.INLINE_PAGE_CONTENT_MAX_CHARS` | `5_000` | `BROWSEROS_LIMIT_INLINE_PAGE_CONTENT_MAX_CHARS` |

#### paths.ts

| Constant | Current | ENV Override |
|---|---|---|
| `SOUL_MAX_LINES` | `150` | `BROWSEROS_LIMIT_SOUL_MAX_LINES` |
| `MEMORY_RETENTION_DAYS` | `30` | `BROWSEROS_LIMIT_MEMORY_RETENTION_DAYS` |
| `SESSION_RETENTION_DAYS` | `30` | `BROWSEROS_LIMIT_SESSION_RETENTION_DAYS` |

### Fix tool-adapter.ts hardcoded timeout

`packages/browseros-agent/apps/server/src/agent/tool-adapter.ts` line with `AbortSignal.timeout(120_000)` must reference `TIMEOUTS.TOOL_CALL` instead.

### Out of Scope

- Ports (already configurable via env/CLI/config)
- Build-time inlined env vars (SENTRY_DSN, etc.)
- External URLs (KLAVIS_PROXY, etc.)
- Per-request config (LLM provider, model, etc.)
- Config file schema changes for `--config server.json` (env vars are sufficient)

## Constraints

1. **Zero breaking changes** — all current hardcoded values remain the defaults
2. **Environment variable only** — no config file schema changes needed
3. **Parse at module load** — env vars read once when the constant module is first imported
4. **Type safety preserved** — the exported types (`TimeoutKey`, etc.) remain the same
5. **`const` assertion** — the TIMEOUTS/LIMITS/PATHS exports may need to drop `as const` since values are now runtime-determined. Alternatively, keep `as const` on the defaults and build the runtime object from them.

## Implementation Approach

For each constant file, the pattern is:

```ts
// Before (hardcoded):
export const TIMEOUTS = {
  TOOL_CALL: 120_000,
  // ...
} as const

// After (env-overridable):
function envMs(name: string, fallback: number): number {
  const raw = process.env[name]
  if (!raw) return fallback
  const parsed = parseInt(raw, 10)
  if (Number.isNaN(parsed) || parsed < 0) return fallback
  return parsed
}

export const TIMEOUTS = {
  TOOL_CALL: envMs('BROWSEROS_TIMEOUT_TOOL_CALL', 120_000),
  // ...
}
```

## Acceptance Criteria

- [ ] All listed constants are overridable via env vars
- [ ] Defaults match current hardcoded values exactly
- [ ] Invalid env var values (NaN, negative) fall back to defaults
- [ ] `tool-adapter.ts` references `TIMEOUTS.TOOL_CALL` instead of hardcoded `120_000`
- [ ] Existing tests pass (no behavioral change without env vars set)
- [ ] New tests cover: env override, invalid fallback, zero/negative rejection
- [ ] Test coverage ≥ 80% on modified files
