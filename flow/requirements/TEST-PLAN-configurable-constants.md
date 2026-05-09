# TEST-PLAN: Configurable Constants Tests

## Parent Requirement
`flow/requirements/REQ-configurable-constants.md`

## Strategy
Write tests FIRST before any implementation. Target ≥ 80% coverage on all modified files.

## Files to Test

### 1. `packages/browseros-agent/packages/shared/src/constants/timeouts.ts`
**New test file:** `packages/browseros-agent/packages/shared/src/constants/__tests__/timeouts.test.ts`

Tests:
- Default values match current hardcoded values (all 25 constants)
- Env var override for each constant (set env, import, check value)
- Invalid env var (non-numeric) falls back to default
- Negative env var falls back to default
- Zero env var is accepted (0 is valid for some)
- Missing env var uses default
- `KLAVIS_PROXY_RETRY_BACKOFF_MS` array is NOT affected by env vars (stays hardcoded)

### 2. `packages/browseros-agent/packages/shared/src/constants/limits.ts`
**New test file:** `packages/browseros-agent/packages/shared/src/constants/__tests__/limits.test.ts`

Tests:
- Default values match current hardcoded values
- `AGENT_LIMITS.MAX_TURNS` override via env
- `AGENT_LIMITS.DEFAULT_CONTEXT_WINDOW` override via env
- `AGENT_LIMITS.COMPACTION_SUMMARIZATION_TIMEOUT_MS` override via env
- `TOOL_LIMITS.FILESYSTEM_READ_MAX_LINES` override via env
- `TOOL_LIMITS.FILESYSTEM_READ_MAX_CHARS` override via env
- `TOOL_LIMITS.INLINE_PAGE_CONTENT_MAX_CHARS` override via env
- Invalid env values fall back to defaults
- Non-overridable limits (compression ratios, compaction thresholds) remain unchanged

### 3. `packages/browseros-agent/packages/shared/src/constants/paths.ts`
**New test file:** `packages/browseros-agent/packages/shared/src/constants/__tests__/paths.test.ts`

Tests:
- Default values match current hardcoded values
- `SOUL_MAX_LINES` override via env
- `MEMORY_RETENTION_DAYS` override via env
- `SESSION_RETENTION_DAYS` override via env
- Invalid env values fall back to defaults
- Directory name constants are NOT affected (stay hardcoded)

### 4. `packages/browseros-agent/apps/server/src/agent/tool-adapter.ts`
**Existing test file may exist. Add tests for:**
- `buildBrowserToolSet` uses `TIMEOUTS.TOOL_CALL` (not hardcoded 120_000)

## Test Pattern

Since env vars are read at module load time, tests must:
1. Delete `require.cache` or use `vi.resetModules()` (vitest) between tests
2. Set `process.env.X = 'value'` before import
3. Import the module fresh
4. Assert the value
5. Clean up env after

```ts
function withEnv(key: string, value: string, fn: () => void) {
  const prev = process.env[key]
  process.env[key] = value
  try {
    fn()
  } finally {
    if (prev === undefined) delete process.env[key]
    else process.env[key] = prev
  }
}
```

## Coverage Target
≥ 80% line coverage on:
- `timeouts.ts`
- `limits.ts`
- `paths.ts`
- `tool-adapter.ts` (for the timeout reference change)
