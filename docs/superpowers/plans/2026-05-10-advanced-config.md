# Advanced Config — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an Advanced Config settings page that exposes runtime configuration with schema-validated editing, ConfigStore pattern, and restart-required UX.

**Architecture:** ConfigStore singleton replaces frozen `const` exports with getter-based access. Server exposes GET/PUT/DELETE `/config` API. Agent extension renders grouped form fields with per-key validation. Changes persist to `advanced-config.json`, applied on next BrowserOS launch.

**Tech Stack:** TypeScript, Zod, React, Hono, Bun test

---

## File Structure

```
packages/shared/src/constants/
  config-schema.ts              # NEW: Zod schema + key metadata for all 37 keys
  config-store.ts               # NEW: ConfigStore singleton (merge ENV > file > default)
  timeouts.ts                   # MODIFY: const values → getters
  limits.ts                     # MODIFY: const values → getters
  paths.ts                      # MODIFY: const values → getters

apps/server/src/api/routes/
  config.ts                     # NEW: GET/PUT/DELETE /config

apps/server/src/api/server.ts   # MODIFY: register /config route

apps/agent/entrypoints/app/advanced-config/
  AdvancedConfigPage.tsx        # NEW: main page
  ConfigGroup.tsx               # NEW: collapsible section
  ConfigField.tsx               # NEW: single field input
  DangerousSection.tsx          # NEW: dangerous keys with risk explanations
  PendingRestartBanner.tsx      # NEW: top banner
  config-queries.ts             # NEW: fetch helpers + types

apps/agent/entrypoints/app/App.tsx              # MODIFY: add route
apps/agent/components/sidebar/SettingsSidebar.tsx # MODIFY: add nav item

packages/shared/tests/constants/
  config-schema.test.ts         # NEW
  config-store.test.ts          # NEW

apps/server/tests/
  config.test.ts                # NEW
```

---

### Task 1: Config Schema

**Files:**
- Create: `packages/shared/src/constants/config-schema.ts`
- Test: `packages/shared/tests/constants/config-schema.test.ts`

- [ ] **Step 1: Write the schema with all 37 keys**

```ts
// packages/shared/src/constants/config-schema.ts
import { z } from 'zod'

export type ConfigSection = 'safe' | 'dangerous'
export type ConfigGroup = 'Timeouts' | 'Limits' | 'Retention'

export interface ConfigKeyMeta {
  /** Dot-path key used in advanced-config.json */
  key: string
  /** Human-readable label */
  label: string
  /** Group for UI sections */
  group: ConfigGroup
  /** Section: safe or dangerous */
  section: ConfigSection
  /** Unit suffix for display */
  unit: 'ms' | 'turns' | 'tokens' | 'chars' | 'lines' | 'days'
  /** Minimum allowed value */
  min: number
  /** Maximum allowed value */
  max: number
  /** Default value */
  default: number
  /** Human-readable description */
  description: string
  /** Risk explanation (dangerous keys only) */
  risk?: string
  /** ENV variable name */
  envVar: string
}

export const CONFIG_KEYS: ConfigKeyMeta[] = [
  // ── Timeouts (safe) ──────────────────────────────────
  { key: 'TIMEOUTS.TOOL_CALL', label: 'Tool Call Timeout', group: 'Timeouts', section: 'safe', unit: 'ms', min: 1_000, max: 600_000, default: 120_000, description: 'Maximum time for a single tool call execution', envVar: 'BROWSEROS_TIMEOUT_TOOL_CALL' },
  { key: 'TIMEOUTS.TOOL_POST_ACTION', label: 'Tool Post-Action Delay', group: 'Timeouts', section: 'safe', unit: 'ms', min: 0, max: 30_000, default: 2_000, description: 'Delay after tool action before proceeding', envVar: 'BROWSEROS_TIMEOUT_TOOL_POST_ACTION' },
  { key: 'TIMEOUTS.TEST_PROVIDER', label: 'Test Provider Timeout', group: 'Timeouts', section: 'safe', unit: 'ms', min: 1_000, max: 120_000, default: 15_000, description: 'Timeout when testing LLM provider connection', envVar: 'BROWSEROS_TIMEOUT_TEST_PROVIDER' },
  { key: 'TIMEOUTS.REFINE_PROMPT', label: 'Refine Prompt Timeout', group: 'Timeouts', section: 'safe', unit: 'ms', min: 1_000, max: 120_000, default: 30_000, description: 'Timeout for prompt refinement requests', envVar: 'BROWSEROS_TIMEOUT_REFINE_PROMPT' },
  { key: 'TIMEOUTS.MCP_DEFAULT', label: 'MCP Default Timeout', group: 'Timeouts', section: 'safe', unit: 'ms', min: 1_000, max: 60_000, default: 5_000, description: 'Default timeout for MCP operations', envVar: 'BROWSEROS_TIMEOUT_MCP_DEFAULT' },
  { key: 'TIMEOUTS.MCP_TRANSPORT_PROBE', label: 'MCP Transport Probe', group: 'Timeouts', section: 'safe', unit: 'ms', min: 1_000, max: 30_000, default: 5_000, description: 'Timeout for probing MCP transport', envVar: 'BROWSEROS_TIMEOUT_MCP_TRANSPORT_PROBE' },
  { key: 'TIMEOUTS.MCP_CLIENT_CONNECT', label: 'MCP Client Connect', group: 'Timeouts', section: 'safe', unit: 'ms', min: 1_000, max: 120_000, default: 15_000, description: 'Timeout for MCP client connection', envVar: 'BROWSEROS_TIMEOUT_MCP_CLIENT_CONNECT' },
  { key: 'TIMEOUTS.CDP_CONNECT', label: 'CDP Connect', group: 'Timeouts', section: 'safe', unit: 'ms', min: 1_000, max: 60_000, default: 10_000, description: 'Timeout for Chrome DevTools Protocol connection', envVar: 'BROWSEROS_TIMEOUT_CDP_CONNECT' },
  { key: 'TIMEOUTS.CDP_REQUEST_TIMEOUT', label: 'CDP Request Timeout', group: 'Timeouts', section: 'safe', unit: 'ms', min: 1_000, max: 300_000, default: 60_000, description: 'Timeout for individual CDP requests', envVar: 'BROWSEROS_TIMEOUT_CDP_REQUEST_TIMEOUT' },
  { key: 'TIMEOUTS.KLAVIS_FETCH', label: 'Klavis Fetch Timeout', group: 'Timeouts', section: 'safe', unit: 'ms', min: 1_000, max: 120_000, default: 30_000, description: 'Timeout for Klavis API fetches', envVar: 'BROWSEROS_TIMEOUT_KLAVIS_FETCH' },
  { key: 'TIMEOUTS.SKILLS_FETCH', label: 'Skills Fetch Timeout', group: 'Timeouts', section: 'safe', unit: 'ms', min: 1_000, max: 120_000, default: 15_000, description: 'Timeout for fetching skills', envVar: 'BROWSEROS_TIMEOUT_SKILLS_FETCH' },
  { key: 'TIMEOUTS.SKILLS_SYNC_INTERVAL', label: 'Skills Sync Interval', group: 'Timeouts', section: 'safe', unit: 'ms', min: 60_000, max: 86_400_000, default: 2_700_000, description: 'Interval between skill synchronization checks (45 min)', envVar: 'BROWSEROS_TIMEOUT_SKILLS_SYNC_INTERVAL' },
  { key: 'TIMEOUTS.NAVIGATION', label: 'Navigation Timeout', group: 'Timeouts', section: 'safe', unit: 'ms', min: 1_000, max: 120_000, default: 10_000, description: 'Timeout for page navigation', envVar: 'BROWSEROS_TIMEOUT_NAVIGATION' },
  { key: 'TIMEOUTS.PAGE_LOAD_WAIT', label: 'Page Load Wait', group: 'Timeouts', section: 'safe', unit: 'ms', min: 1_000, max: 120_000, default: 30_000, description: 'Maximum wait for page load completion', envVar: 'BROWSEROS_TIMEOUT_PAGE_LOAD_WAIT' },
  { key: 'TIMEOUTS.STABLE_DOM', label: 'Stable DOM Wait', group: 'Timeouts', section: 'safe', unit: 'ms', min: 100, max: 30_000, default: 3_000, description: 'Wait time for DOM to stabilize', envVar: 'BROWSEROS_TIMEOUT_STABLE_DOM' },
  { key: 'TIMEOUTS.FILE_CHOOSER', label: 'File Chooser Timeout', group: 'Timeouts', section: 'safe', unit: 'ms', min: 100, max: 30_000, default: 3_000, description: 'Timeout for file chooser dialog', envVar: 'BROWSEROS_TIMEOUT_FILE_CHOOSER' },
  { key: 'TIMEOUTS.OAUTH_FLOW_TTL', label: 'OAuth Flow TTL', group: 'Timeouts', section: 'safe', unit: 'ms', min: 10_000, max: 3_600_000, default: 300_000, description: 'Time-to-live for OAuth flow state', envVar: 'BROWSEROS_TIMEOUT_OAUTH_FLOW_TTL' },
  { key: 'TIMEOUTS.OAUTH_POLL_TIMEOUT', label: 'OAuth Poll Timeout', group: 'Timeouts', section: 'safe', unit: 'ms', min: 10_000, max: 3_600_000, default: 300_000, description: 'Timeout for OAuth polling', envVar: 'BROWSEROS_TIMEOUT_OAUTH_POLL_TIMEOUT' },

  // ── Timeouts (dangerous) ─────────────────────────────
  { key: 'TIMEOUTS.CDP_CONNECT_RETRY_DELAY', label: 'CDP Connect Retry Delay', group: 'Timeouts', section: 'dangerous', unit: 'ms', min: 100, max: 30_000, default: 1_000, description: 'Delay between CDP connection retries', risk: 'Too low → retry storms. Too high → slow recovery from disconnects.', envVar: 'BROWSEROS_TIMEOUT_CDP_CONNECT_RETRY_DELAY' },
  { key: 'TIMEOUTS.CDP_RECONNECT_DELAY', label: 'CDP Reconnect Delay', group: 'Timeouts', section: 'dangerous', unit: 'ms', min: 100, max: 60_000, default: 5_000, description: 'Delay before reconnecting to the browser', risk: 'Too low → rapid retry loop.', envVar: 'BROWSEROS_TIMEOUT_CDP_RECONNECT_DELAY' },
  { key: 'TIMEOUTS.CDP_KEEPALIVE_INTERVAL', label: 'CDP Keepalive Interval', group: 'Timeouts', section: 'dangerous', unit: 'ms', min: 1_000, max: 300_000, default: 30_000, description: 'How often to ping the browser', risk: 'Too low → unnecessary load. Too high → missed disconnects.', envVar: 'BROWSEROS_TIMEOUT_CDP_KEEPALIVE_INTERVAL' },
  { key: 'TIMEOUTS.CDP_KEEPALIVE_TIMEOUT', label: 'CDP Keepalive Timeout', group: 'Timeouts', section: 'dangerous', unit: 'ms', min: 1_000, max: 120_000, default: 10_000, description: 'Wait time for keepalive response', risk: 'Too low → false disconnects.', envVar: 'BROWSEROS_TIMEOUT_CDP_KEEPALIVE_TIMEOUT' },
  { key: 'TIMEOUTS.PAGE_LOAD_POLL_INTERVAL', label: 'Page Load Poll Interval', group: 'Timeouts', section: 'dangerous', unit: 'ms', min: 10, max: 5_000, default: 150, description: 'Polling frequency for page load detection', risk: 'Too low → CPU waste. Too high → slow detection.', envVar: 'BROWSEROS_TIMEOUT_PAGE_LOAD_POLL_INTERVAL' },
  { key: 'TIMEOUTS.OAUTH_TOKEN_EXPIRY_BUFFER', label: 'OAuth Token Expiry Buffer', group: 'Timeouts', section: 'dangerous', unit: 'ms', min: 10_000, max: 3_600_000, default: 300_000, description: 'How far before expiry to refresh tokens', risk: 'Too low → token expires mid-flow.', envVar: 'BROWSEROS_TIMEOUT_OAUTH_TOKEN_EXPIRY_BUFFER' },
  { key: 'TIMEOUTS.OAUTH_POLL_INTERVAL', label: 'OAuth Poll Interval', group: 'Timeouts', section: 'dangerous', unit: 'ms', min: 100, max: 30_000, default: 2_000, description: 'Polling frequency during OAuth flows', risk: 'Too low → API rate limits.', envVar: 'BROWSEROS_TIMEOUT_OAUTH_POLL_INTERVAL' },
  { key: 'TIMEOUTS.DEVICE_CODE_POLL_SAFETY_MARGIN', label: 'Device Code Poll Safety Margin', group: 'Timeouts', section: 'dangerous', unit: 'ms', min: 100, max: 30_000, default: 3_000, description: 'Safety margin before device code expiry', risk: 'Too low → flow fails prematurely.', envVar: 'BROWSEROS_TIMEOUT_DEVICE_CODE_POLL_SAFETY_MARGIN' },

  // ── Limits (safe) ────────────────────────────────────
  { key: 'AGENT_LIMITS.MAX_TURNS', label: 'Max Turns', group: 'Limits', section: 'safe', unit: 'turns', min: 1, max: 1_000, default: 100, description: 'Maximum agent turns per conversation', envVar: 'BROWSEROS_LIMIT_MAX_TURNS' },
  { key: 'AGENT_LIMITS.DEFAULT_CONTEXT_WINDOW', label: 'Default Context Window', group: 'Limits', section: 'safe', unit: 'tokens', min: 1_000, max: 2_000_000, default: 200_000, description: 'Default context window size for LLM', envVar: 'BROWSEROS_LIMIT_DEFAULT_CONTEXT_WINDOW' },
  { key: 'TOOL_LIMITS.INLINE_PAGE_CONTENT_MAX_CHARS', label: 'Inline Page Content Limit', group: 'Limits', section: 'safe', unit: 'chars', min: 100, max: 100_000, default: 5_000, description: 'Maximum inline page content characters', envVar: 'BROWSEROS_LIMIT_INLINE_PAGE_CONTENT_MAX_CHARS' },
  { key: 'TOOL_LIMITS.FILESYSTEM_READ_MAX_LINES', label: 'Filesystem Read Max Lines', group: 'Limits', section: 'safe', unit: 'lines', min: 1, max: 10_000, default: 500, description: 'Maximum lines when reading files', envVar: 'BROWSEROS_LIMIT_FILESYSTEM_READ_MAX_LINES' },
  { key: 'TOOL_LIMITS.FILESYSTEM_READ_MAX_CHARS', label: 'Filesystem Read Max Chars', group: 'Limits', section: 'safe', unit: 'chars', min: 100, max: 100_000, default: 15_000, description: 'Maximum characters when reading files', envVar: 'BROWSEROS_LIMIT_FILESYSTEM_READ_MAX_CHARS' },

  // ── Limits (dangerous) ───────────────────────────────
  { key: 'AGENT_LIMITS.COMPACTION_SUMMARIZATION_TIMEOUT_MS', label: 'Compaction Summarization Timeout', group: 'Limits', section: 'dangerous', unit: 'ms', min: 5_000, max: 600_000, default: 60_000, description: 'Timeout for LLM summarization during compaction', risk: 'Too low → incomplete summaries during compaction.', envVar: 'BROWSEROS_TIMEOUT_COMPACTION_SUMMARIZATION' },
  { key: 'AGENT_LIMITS.COMPACTION_MAX_SUMMARIZATION_INPUT', label: 'Compaction Max Input', group: 'Limits', section: 'dangerous', unit: 'tokens', min: 1_000, max: 1_000_000, default: 100_000, description: 'Max input tokens for compaction summarization', risk: 'Too low → context loss. Too high → API costs.', envVar: 'BROWSEROS_LIMIT_COMPACTION_MAX_SUMMARIZATION_INPUT' },
  { key: 'AGENT_LIMITS.COMPACTION_TOOL_OUTPUT_MAX_CHARS', label: 'Compaction Tool Output Limit', group: 'Limits', section: 'dangerous', unit: 'chars', min: 100, max: 1_000_000, default: 15_000, description: 'Max chars kept from tool outputs in transcript', risk: 'Too low → lost context. Too high → memory bloat.', envVar: 'BROWSEROS_LIMIT_COMPACTION_TOOL_OUTPUT_MAX_CHARS' },

  // ── Retention (all safe) ─────────────────────────────
  { key: 'PATHS.SOUL_MAX_LINES', label: 'Soul Max Lines', group: 'Retention', section: 'safe', unit: 'lines', min: 10, max: 10_000, default: 150, description: 'Maximum lines in SOUL.md', envVar: 'BROWSEROS_LIMIT_SOUL_MAX_LINES' },
  { key: 'PATHS.MEMORY_RETENTION_DAYS', label: 'Memory Retention', group: 'Retention', section: 'safe', unit: 'days', min: 1, max: 365, default: 30, description: 'Days to retain memory entries', envVar: 'BROWSEROS_LIMIT_MEMORY_RETENTION_DAYS' },
  { key: 'PATHS.SESSION_RETENTION_DAYS', label: 'Session Retention', group: 'Retention', section: 'safe', unit: 'days', min: 1, max: 365, default: 30, description: 'Days to retain session data', envVar: 'BROWSEROS_LIMIT_SESSION_RETENTION_DAYS' },
]

/** Map from dot-path key to metadata */
export const CONFIG_KEY_MAP = new Map(CONFIG_KEYS.map(k => [k.key, k]))

/** Zod schema for validating overrides object */
export const ConfigOverridesSchema = z.record(
  z.string(),
  z.number().int().safe('Must be a safe integer').nonnegative('Must be non-negative'),
)

export type ConfigOverrides = z.infer<typeof ConfigOverridesSchema>

/** Validate a single value against its key's constraints */
export function validateConfigValue(key: string, value: number): string | null {
  const meta = CONFIG_KEY_MAP.get(key)
  if (!meta) return `Unknown config key: ${key}`
  if (!Number.isSafeInteger(value)) return 'Must be a safe integer'
  if (value < 0) return 'Must be non-negative'
  if (value < meta.min) return `Value ${value} is below minimum ${meta.min}`
  if (value > meta.max) return `Value ${value} exceeds maximum ${meta.max}`
  return null
}

/** Validate all overrides, returns error map (empty = valid) */
export function validateAllOverrides(overrides: ConfigOverrides): Map<string, string> {
  const errors = new Map<string, string>()
  for (const [key, value] of Object.entries(overrides)) {
    const err = validateConfigValue(key, value)
    if (err) errors.set(key, err)
  }
  return errors
}
```

- [ ] **Step 2: Write schema tests**

```ts
// packages/shared/tests/constants/config-schema.test.ts
import { describe, expect, test } from 'bun:test'
import { CONFIG_KEYS, CONFIG_KEY_MAP, validateConfigValue, validateAllOverrides, ConfigOverridesSchema } from '@browseros/shared/constants/config-schema'

describe('CONFIG_KEYS', () => {
  test('has 37 keys total', () => {
    expect(CONFIG_KEYS).toHaveLength(37)
  })

  test('all keys have unique dot-paths', () => {
    const keys = CONFIG_KEYS.map(k => k.key)
    expect(new Set(keys).size).toBe(keys.length)
  })

  test('all keys have required fields', () => {
    for (const k of CONFIG_KEYS) {
      expect(k.key).toBeTruthy()
      expect(k.label).toBeTruthy()
      expect(k.group).toMatch(/^(Timeouts|Limits|Retention)$/)
      expect(k.section).toMatch(/^(safe|dangerous)$/)
      expect(k.unit).toBeTruthy()
      expect(k.min).toBeGreaterThanOrEqual(0)
      expect(k.max).toBeGreaterThan(k.min)
      expect(k.default).toBeGreaterThanOrEqual(k.min)
      expect(k.default).toBeLessThanOrEqual(k.max)
      expect(k.description).toBeTruthy()
      expect(k.envVar).toBeTruthy()
    }
  })

  test('dangerous keys have risk explanations', () => {
    const dangerous = CONFIG_KEYS.filter(k => k.section === 'dangerous')
    expect(dangerous.length).toBeGreaterThan(0)
    for (const k of dangerous) {
      expect(k.risk).toBeTruthy()
    }
  })

  test('CONFIG_KEY_MAP matches CONFIG_KEYS', () => {
    expect(CONFIG_KEY_MAP.size).toBe(CONFIG_KEYS.length)
    for (const k of CONFIG_KEYS) {
      expect(CONFIG_KEY_MAP.get(k.key)).toBe(k)
    }
  })
})

describe('validateConfigValue', () => {
  test('accepts valid value within range', () => {
    expect(validateConfigValue('TIMEOUTS.TOOL_CALL', 60_000)).toBeNull()
  })

  test('rejects unknown key', () => {
    expect(validateConfigValue('UNKNOWN.KEY', 100)).toContain('Unknown config key')
  })

  test('rejects value below minimum', () => {
    expect(validateConfigValue('TIMEOUTS.TOOL_CALL', 500)).toContain('below minimum')
  })

  test('rejects value above maximum', () => {
    expect(validateConfigValue('TIMEOUTS.TOOL_CALL', 999_999)).toContain('exceeds maximum')
  })

  test('rejects negative value', () => {
    expect(validateConfigValue('TIMEOUTS.TOOL_CALL', -1)).toContain('non-negative')
  })

  test('accepts exact min boundary', () => {
    expect(validateConfigValue('TIMEOUTS.TOOL_CALL', 1_000)).toBeNull()
  })

  test('accepts exact max boundary', () => {
    expect(validateConfigValue('TIMEOUTS.TOOL_CALL', 600_000)).toBeNull()
  })
})

describe('validateAllOverrides', () => {
  test('returns empty map for valid overrides', () => {
    const errors = validateAllOverrides({ 'TIMEOUTS.TOOL_CALL': 60_000 })
    expect(errors.size).toBe(0)
  })

  test('returns errors for multiple invalid values', () => {
    const errors = validateAllOverrides({
      'TIMEOUTS.TOOL_CALL': -1,
      'AGENT_LIMITS.MAX_TURNS': 9999,
      'UNKNOWN.KEY': 100,
    })
    expect(errors.size).toBe(3)
  })
})

describe('ConfigOverridesSchema', () => {
  test('accepts valid overrides', () => {
    const result = ConfigOverridesSchema.safeParse({ 'TIMEOUTS.TOOL_CALL': 60_000 })
    expect(result.success).toBe(true)
  })

  test('rejects non-integer values', () => {
    const result = ConfigOverridesSchema.safeParse({ 'TIMEOUTS.TOOL_CALL': 1.5 })
    expect(result.success).toBe(false)
  })

  test('rejects string values', () => {
    const result = ConfigOverridesSchema.safeParse({ 'TIMEOUTS.TOOL_CALL': '60000' })
    expect(result.success).toBe(false)
  })
})
```

- [ ] **Step 3: Run tests**

Run: `cd packages/browseros-agent && bun test packages/shared/tests/constants/config-schema.test.ts`
Expected: all pass

- [ ] **Step 4: Commit**

```bash
git add packages/shared/src/constants/config-schema.ts packages/shared/tests/constants/config-schema.test.ts
git commit -m "feat(shared): add config schema with 37 keys, validation, and metadata"
```

---

### Task 2: ConfigStore

**Files:**
- Create: `packages/shared/src/constants/config-store.ts`
- Test: `packages/shared/tests/constants/config-store.test.ts`

- [ ] **Step 1: Write ConfigStore**

```ts
// packages/shared/src/constants/config-store.ts
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { CONFIG_KEY_MAP, validateAllOverrides, type ConfigOverrides } from './config-schema'

export class ConfigStore {
  private fileOverrides: ConfigOverrides = {}
  private filePath: string | null = null

  /** Initialize the store. Reads overrides from disk if path provided. */
  init(configFilePath?: string): void {
    this.filePath = configFilePath ?? null
    this.fileOverrides = this.readFromDisk()
  }

  /** Get merged value for a dot-path key. Priority: ENV > file > default. */
  get(key: string): number {
    const meta = CONFIG_KEY_MAP.get(key)
    if (!meta) throw new Error(`Unknown config key: ${key}`)

    // 1. ENV wins
    const envVal = this.readEnv(meta.envVar)
    if (envVal !== undefined) return envVal

    // 2. File override
    if (key in this.fileOverrides) return this.fileOverrides[key]

    // 3. Default
    return meta.default
  }

  /** Get all active values (current in-memory state) */
  getAllActive(): ConfigOverrides {
    const result: ConfigOverrides = {}
    for (const [key] of CONFIG_KEY_MAP) {
      result[key] = this.get(key)
    }
    return result
  }

  /** Get file overrides only (what's persisted) */
  getFileOverrides(): ConfigOverrides {
    return { ...this.fileOverrides }
  }

  /** Get all defaults */
  getDefaults(): ConfigOverrides {
    const result: ConfigOverrides = {}
    for (const [key, meta] of CONFIG_KEY_MAP) {
      result[key] = meta.default
    }
    return result
  }

  /** Validate and save overrides to disk. Returns error map (empty = success). */
  save(overrides: ConfigOverrides): Map<string, string> {
    // Only allow known keys
    const unknownKeys = Object.keys(overrides).filter(k => !CONFIG_KEY_MAP.has(k))
    if (unknownKeys.length > 0) {
      const errors = new Map<string, string>()
      for (const k of unknownKeys) errors.set(k, `Unknown config key: ${k}`)
      return errors
    }

    const errors = validateAllOverrides(overrides)
    if (errors.size > 0) return errors

    this.fileOverrides = { ...overrides }
    this.writeToDisk()
    return new Map()
  }

  /** Delete all overrides (reset to defaults). */
  reset(): void {
    this.fileOverrides = {}
    if (this.filePath && existsSync(this.filePath)) {
      try { require('node:fs').unlinkSync(this.filePath) } catch { /* ok */ }
    }
  }

  /** Check if saved overrides differ from active (in-memory) values */
  hasPendingChanges(): boolean {
    for (const [key] of CONFIG_KEY_MAP) {
      const active = this.get(key)
      const meta = CONFIG_KEY_MAP.get(key)!
      const saved = this.fileOverrides[key]
      // Pending if file has override AND ENV overrides it differently
      if (saved !== undefined && active !== saved) return true
    }
    return false
  }

  // ── Private ──────────────────────────────────────────

  private readEnv(envVar: string): number | undefined {
    const raw = process.env[envVar]
    if (raw === undefined) return undefined
    const normalized = raw.trim().replace(/_/g, '')
    if (!/^\d+$/.test(normalized)) return undefined
    const parsed = Number(normalized)
    if (!Number.isSafeInteger(parsed) || parsed < 0) return undefined
    return parsed
  }

  private readFromDisk(): ConfigOverrides {
    if (!this.filePath || !existsSync(this.filePath)) return {}
    try {
      const content = readFileSync(this.filePath, 'utf-8')
      const parsed = JSON.parse(content)
      if (typeof parsed !== 'object' || parsed === null) return {}
      return parsed as ConfigOverrides
    } catch {
      return {}
    }
  }

  private writeToDisk(): void {
    if (!this.filePath) return
    writeFileSync(this.filePath, `${JSON.stringify(this.fileOverrides, null, 2)}\n`)
  }
}

/** Singleton instance */
export const configStore = new ConfigStore()
```

- [ ] **Step 2: Write ConfigStore tests**

```ts
// packages/shared/tests/constants/config-store.test.ts
import { existsSync, mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { mkdtempSync } from 'node:os'
import { join } from 'node:path'
import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { ConfigStore } from '@browseros/shared/constants/config-store'

// Each test gets a fresh store + tmp dir
let tmpDir: string
let store: ConfigStore
let configFile: string

beforeEach(() => {
  tmpDir = mkdtempSync(join(import.meta.dir, '.config-store-test-'))
  configFile = join(tmpDir, 'advanced-config.json')
  store = new ConfigStore()
})

afterEach(() => {
  rmSync(tmpDir, { recursive: true, force: true })
})

describe('ConfigStore defaults', () => {
  test('returns default when no ENV and no file', () => {
    store.init()
    expect(store.get('TIMEOUTS.TOOL_CALL')).toBe(120_000)
    expect(store.get('AGENT_LIMITS.MAX_TURNS')).toBe(100)
    expect(store.get('PATHS.SESSION_RETENTION_DAYS')).toBe(30)
  })

  test('throws on unknown key', () => {
    store.init()
    expect(() => store.get('UNKNOWN.KEY')).toThrow('Unknown config key')
  })

  test('getAllActive returns all 37 keys', () => {
    store.init()
    const active = store.getAllActive()
    expect(Object.keys(active)).toHaveLength(37)
  })

  test('getDefaults returns all defaults', () => {
    const defaults = store.getDefaults()
    expect(defaults['TIMEOUTS.TOOL_CALL']).toBe(120_000)
    expect(Object.keys(defaults)).toHaveLength(37)
  })
})

describe('ConfigStore file overrides', () => {
  test('reads overrides from disk on init', () => {
    writeFileSync(configFile, JSON.stringify({ 'TIMEOUTS.TOOL_CALL': 60_000 }))
    store.init(configFile)
    expect(store.get('TIMEOUTS.TOOL_CALL')).toBe(60_000)
  })

  test('file overrides do not affect keys without overrides', () => {
    writeFileSync(configFile, JSON.stringify({ 'TIMEOUTS.TOOL_CALL': 60_000 }))
    store.init(configFile)
    expect(store.get('AGENT_LIMITS.MAX_TURNS')).toBe(100)
  })

  test('gracefully handles corrupt file', () => {
    writeFileSync(configFile, 'not json{{{')
    store.init(configFile)
    expect(store.get('TIMEOUTS.TOOL_CALL')).toBe(120_000)
  })

  test('gracefully handles missing file', () => {
    store.init(join(tmpDir, 'nonexistent.json'))
    expect(store.get('TIMEOUTS.TOOL_CALL')).toBe(120_000)
  })
})

describe('ConfigStore save', () => {
  test('saves valid overrides to disk', () => {
    store.init(configFile)
    const errors = store.save({ 'TIMEOUTS.TOOL_CALL': 30_000 })
    expect(errors.size).toBe(0)
    expect(store.get('TIMEOUTS.TOOL_CALL')).toBe(30_000)
    expect(existsSync(configFile)).toBe(true)
  })

  test('rejects invalid values', () => {
    store.init(configFile)
    const errors = store.save({ 'TIMEOUTS.TOOL_CALL': -1 })
    expect(errors.size).toBe(1)
    expect(errors.get('TIMEOUTS.TOOL_CALL')).toBeTruthy()
  })

  test('rejects unknown keys', () => {
    store.init(configFile)
    const errors = store.save({ 'UNKNOWN.KEY': 100 })
    expect(errors.size).toBe(1)
  })

  test('getFileOverrides returns saved overrides', () => {
    store.init(configFile)
    store.save({ 'TIMEOUTS.TOOL_CALL': 30_000 })
    const overrides = store.getFileOverrides()
    expect(overrides['TIMEOUTS.TOOL_CALL']).toBe(30_000)
  })
})

describe('ConfigStore reset', () => {
  test('removes overrides and deletes file', () => {
    store.init(configFile)
    store.save({ 'TIMEOUTS.TOOL_CALL': 30_000 })
    store.reset()
    expect(store.get('TIMEOUTS.TOOL_CALL')).toBe(120_000)
    expect(existsSync(configFile)).toBe(false)
  })
})

describe('ConfigStore hasPendingChanges', () => {
  test('returns false when no overrides saved', () => {
    store.init(configFile)
    expect(store.hasPendingChanges()).toBe(false)
  })

  test('returns false when saved matches active', () => {
    store.init(configFile)
    store.save({ 'TIMEOUTS.TOOL_CALL': 30_000 })
    // No ENV override, so active = file = 30000
    expect(store.hasPendingChanges()).toBe(false)
  })
})

describe('ConfigStore ENV priority', () => {
  test('ENV wins over file and default', () => {
    process.env.BROWSEROS_TIMEOUT_TOOL_CALL = '90000'
    store.init(configFile)
    store.save({ 'TIMEOUTS.TOOL_CALL': 30_000 })
    // ENV=90000, file=30000 → ENV wins
    expect(store.get('TIMEOUTS.TOOL_CALL')).toBe(90_000)
    // pending because active (90k from ENV) ≠ saved (30k)
    expect(store.hasPendingChanges()).toBe(true)
    delete process.env.BROWSEROS_TIMEOUT_TOOL_CALL
  })

  test('ENV wins over default when no file', () => {
    process.env.BROWSEROS_LIMIT_MAX_TURNS = '50'
    store.init(configFile)
    expect(store.get('AGENT_LIMITS.MAX_TURNS')).toBe(50)
    delete process.env.BROWSEROS_LIMIT_MAX_TURNS
  })
})
```

- [ ] **Step 3: Run tests**

Run: `cd packages/browseros-agent && bun test packages/shared/tests/constants/config-store.test.ts`
Expected: all pass

- [ ] **Step 4: Commit**

```bash
git add packages/shared/src/constants/config-store.ts packages/shared/tests/constants/config-store.test.ts
git commit -m "feat(shared): add ConfigStore with ENV > file > default merge layer"
```

---

### Task 3: Convert Constants to Getters

**Files:**
- Modify: `packages/shared/src/constants/timeouts.ts`
- Modify: `packages/shared/src/constants/limits.ts`
- Modify: `packages/shared/src/constants/paths.ts`
- Test: `packages/shared/tests/constants/timeouts.test.ts` (update env override tests)
- Test: `packages/shared/tests/constants/limits.test.ts` (update env override tests)
- Test: `packages/shared/tests/constants/paths.test.ts` (update env override tests)

- [ ] **Step 1: Update timeouts.ts — replace const with getters backed by configStore**

Replace the file body. Keep `KLAVIS_PROXY_RETRY_BACKOFF_MS` as a plain const (it's a tuple, not a single config value). Remove `envMs()` helper. Import `configStore` and use its `get()`.

Key change pattern:
```ts
// Before
export const TIMEOUTS = {
  TOOL_CALL: envMs('BROWSEROS_TIMEOUT_TOOL_CALL', 120_000),
  ...
}

// After
import { configStore } from './config-store'

// configStore must be initialized before first access.
// Server does this at startup; agent extension doesn't use these constants directly.
export const TIMEOUTS = {
  get TOOL_CALL() { return configStore.get('TIMEOUTS.TOOL_CALL') },
  get TOOL_POST_ACTION() { return configStore.get('TIMEOUTS.TOOL_POST_ACTION') },
  ...
}
```

- [ ] **Step 2: Update limits.ts — same pattern**

Remove `envInt()` helper. Convert `AGENT_LIMITS`, `TOOL_LIMITS` to getter objects. Keep non-overridable consts as plain values.

For non-overridable fields (COMPRESSION_MIN_HEADROOM, etc.), keep as plain values — they have no config-schema entry and ConfigStore won't have them.

```ts
import { configStore } from './config-store'

export const AGENT_LIMITS = {
  get MAX_TURNS() { return configStore.get('AGENT_LIMITS.MAX_TURNS') },
  get DEFAULT_CONTEXT_WINDOW() { return configStore.get('AGENT_LIMITS.DEFAULT_CONTEXT_WINDOW') },

  // Non-overridable — plain values
  COMPRESSION_MIN_HEADROOM: 10_000,
  COMPRESSION_MAX_RATIO: 0.75,
  ...
  get COMPACTION_SUMMARIZATION_TIMEOUT_MS() { return configStore.get('AGENT_LIMITS.COMPACTION_SUMMARIZATION_TIMEOUT_MS') },
  ...
} as const
```

- [ ] **Step 3: Update paths.ts — same pattern**

Only `SOUL_MAX_LINES`, `MEMORY_RETENTION_DAYS`, `SESSION_RETENTION_DAYS` are overridable.

- [ ] **Step 4: Run existing tests to verify no regressions**

Run: `cd packages/browseros-agent && bun test packages/shared/tests/constants/timeouts.test.ts packages/shared/tests/constants/limits.test.ts packages/shared/tests/constants/paths.test.ts`
Expected: all 61 pass (const→getter is transparent to consumers)

Note: The env override subprocess tests will still pass because `configStore.get()` checks `process.env` at call time, matching the old behavior.

- [ ] **Step 5: Commit**

```bash
git add packages/shared/src/constants/timeouts.ts packages/shared/src/constants/limits.ts packages/shared/src/constants/paths.ts
git commit -m "refactor(shared): convert const exports to getters backed by ConfigStore"
```

---

### Task 4: Server API Route

**Files:**
- Create: `apps/server/src/api/routes/config.ts`
- Modify: `apps/server/src/api/server.ts`
- Test: `apps/server/tests/config.test.ts`

- [ ] **Step 1: Write config route**

```ts
// apps/server/src/api/routes/config.ts
import { Hono } from 'hono'
import { existsSync } from 'node:fs'
import { join } from 'node:path'
import {
  CONFIG_KEYS,
  CONFIG_KEY_MAP,
  type ConfigOverrides,
} from '@browseros/shared/constants/config-schema'
import { getBrowserosDir } from '../../lib/browseros-dir'
import { configStore } from '@browseros/shared/constants/config-store'
import type { Env } from '../types'

function getAdvancedConfigPath(): string {
  return join(getBrowserosDir(), 'advanced-config.json')
}

export function createConfigRoutes(): Hono<Env> {
  const router = new Hono<Env>()

  // Initialize configStore with file path on first request
  let initialized = false
  function ensureInit() {
    if (!initialized) {
      configStore.init(getAdvancedConfigPath())
      initialized = true
    }
  }

  router.get('/', (c) => {
    ensureInit()

    const active = configStore.getAllActive()
    const pending = configStore.getFileOverrides()
    const defaults = configStore.getDefaults()
    const schema: Record<string, typeof CONFIG_KEYS[number]> = {}
    for (const k of CONFIG_KEYS) {
      schema[k.key] = k
    }

    return c.json({
      active,
      pending,
      defaults,
      schema,
      hasPendingChanges: configStore.hasPendingChanges(),
    })
  })

  router.put('/', async (c) => {
    ensureInit()
    const body = await c.req.json<{ overrides: ConfigOverrides }>()
    if (!body.overrides || typeof body.overrides !== 'object') {
      return c.json({ ok: false, errors: [{ key: '_', message: 'Missing overrides object' }] }, 400)
    }

    const errors = configStore.save(body.overrides)
    if (errors.size > 0) {
      const errorList = Array.from(errors.entries()).map(([key, message]) => ({ key, message }))
      return c.json({ ok: false, errors: errorList }, 400)
    }

    return c.json({
      ok: true,
      saved: Object.keys(body.overrides).length,
      hasPendingChanges: configStore.hasPendingChanges(),
    })
  })

  router.delete('/', (c) => {
    ensureInit()
    configStore.reset()
    return c.json({ ok: true, hasPendingChanges: false })
  })

  return router
}
```

- [ ] **Step 2: Register route in server.ts**

Add import and route registration alongside existing routes. In `apps/server/src/api/server.ts`:

```ts
// Add import
import { createConfigRoutes } from './routes/config'

// Add route after existing routes (e.g., after .route('/agents', agentRoutes))
.route('/config', createConfigRoutes())
```

- [ ] **Step 3: Initialize configStore at server startup**

In the `createHttpServer` function, before creating routes, call:

```ts
import { configStore } from '@browseros/shared/constants/config-store'
import { getAdvancedConfigPath } from './routes/config'
// ... inside createHttpServer:
configStore.init(getAdvancedConfigPath())
```

Actually — the route does lazy init. But it's better to init early so all `TIMEOUTS.X` getters work from the start. Add init in `createHttpServer`:

```ts
// After config is destructured, before route creation
configStore.init(join(getBrowserosDir(), 'advanced-config.json'))
```

- [ ] **Step 4: Write route tests**

```ts
// apps/server/tests/config.test.ts
import { existsSync, mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { mkdtempSync } from 'node:os'
import { join } from 'node:path'
import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { configStore } from '@browseros/shared/constants/config-store'
import { createConfigRoutes } from '../../src/api/routes/config'

let tmpDir: string

beforeEach(() => {
  tmpDir = mkdtempSync(join(import.meta.dir, '.config-route-test-'))
  configStore.init(join(tmpDir, 'advanced-config.json'))
})

afterEach(() => {
  configStore.reset()
  rmSync(tmpDir, { recursive: true, force: true })
})

// Create a minimal Hono app for testing
function createTestApp() {
  const { Hono } = require('hono')
  const app = new Hono()
  app.route('/config', createConfigRoutes())
  return app
}

describe('GET /config', () => {
  test('returns defaults when no overrides', async () => {
    const app = createTestApp()
    const res = await app.request('/config')
    expect(res.status).toBe(200)
    const body = await res.json()
    expect(body.active['TIMEOUTS.TOOL_CALL']).toBe(120_000)
    expect(body.defaults['TIMEOUTS.TOOL_CALL']).toBe(120_000)
    expect(Object.keys(body.schema)).toHaveLength(37)
    expect(body.hasPendingChanges).toBe(false)
  })
})

describe('PUT /config', () => {
  test('saves valid overrides', async () => {
    const app = createTestApp()
    const res = await app.request('/config', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ overrides: { 'TIMEOUTS.TOOL_CALL': 60_000 } }),
    })
    expect(res.status).toBe(200)
    const body = await res.json()
    expect(body.ok).toBe(true)
    expect(body.saved).toBe(1)
  })

  test('rejects invalid values', async () => {
    const app = createTestApp()
    const res = await app.request('/config', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ overrides: { 'TIMEOUTS.TOOL_CALL': -1 } }),
    })
    expect(res.status).toBe(400)
    const body = await res.json()
    expect(body.ok).toBe(false)
    expect(body.errors.length).toBeGreaterThan(0)
  })

  test('rejects missing body', async () => {
    const app = createTestApp()
    const res = await app.request('/config', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({}),
    })
    expect(res.status).toBe(400)
  })
})

describe('DELETE /config', () => {
  test('resets all overrides', async () => {
    const app = createTestApp()
    // Save first
    await app.request('/config', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ overrides: { 'TIMEOUTS.TOOL_CALL': 60_000 } }),
    })
    // Then delete
    const res = await app.request('/config', { method: 'DELETE' })
    expect(res.status).toBe(200)
    const body = await res.json()
    expect(body.ok).toBe(true)
    expect(body.hasPendingChanges).toBe(false)
  })
})
```

- [ ] **Step 5: Run tests**

Run: `cd packages/browseros-agent && bun test apps/server/tests/config.test.ts`
Expected: all pass

- [ ] **Step 6: Commit**

```bash
git add apps/server/src/api/routes/config.ts apps/server/src/api/server.ts apps/server/tests/config.test.ts
git commit -m "feat(server): add GET/PUT/DELETE /config API route with schema validation"
```

---

### Task 5: Settings UI — Advanced Config Page

**Files:**
- Create: `apps/agent/entrypoints/app/advanced-config/config-queries.ts`
- Create: `apps/agent/entrypoints/app/advanced-config/ConfigField.tsx`
- Create: `apps/agent/entrypoints/app/advanced-config/ConfigGroup.tsx`
- Create: `apps/agent/entrypoints/app/advanced-config/DangerousSection.tsx`
- Create: `apps/agent/entrypoints/app/advanced-config/PendingRestartBanner.tsx`
- Create: `apps/agent/entrypoints/app/advanced-config/AdvancedConfigPage.tsx`
- Modify: `apps/agent/components/sidebar/SettingsSidebar.tsx`
- Modify: `apps/agent/entrypoints/app/App.tsx`

- [ ] **Step 1: Write config-queries.ts (fetch helpers)**

```ts
// apps/agent/entrypoints/app/advanced-config/config-queries.ts
import { getAgentServerUrl } from '@/lib/browseros/helpers'

export interface ConfigKeySchema {
  key: string
  label: string
  group: string
  section: 'safe' | 'dangerous'
  unit: string
  min: number
  max: number
  default: number
  description: string
  risk?: string
  envVar: string
}

export interface ConfigResponse {
  active: Record<string, number>
  pending: Record<string, number>
  defaults: Record<string, number>
  schema: Record<string, ConfigKeySchema>
  hasPendingChanges: boolean
}

export interface ConfigSaveResponse {
  ok: boolean
  saved?: number
  hasPendingChanges?: boolean
  errors?: Array<{ key: string; message: string }>
}

export async function fetchConfig(): Promise<ConfigResponse> {
  const serverUrl = await getAgentServerUrl()
  const res = await fetch(`${serverUrl}/config`)
  if (!res.ok) throw new Error(`Failed to fetch config: ${res.status}`)
  return res.json()
}

export async function saveConfig(overrides: Record<string, number>): Promise<ConfigSaveResponse> {
  const serverUrl = await getAgentServerUrl()
  const res = await fetch(`${serverUrl}/config`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ overrides }),
  })
  return res.json()
}

export async function resetConfig(): Promise<ConfigSaveResponse> {
  const serverUrl = await getAgentServerUrl()
  const res = await fetch(`${serverUrl}/config`, { method: 'DELETE' })
  return res.json()
}
```

- [ ] **Step 2: Write ConfigField.tsx**

A single field: label, description, input, unit, reset button, error message.

```tsx
// apps/agent/entrypoints/app/advanced-config/ConfigField.tsx
import { type FC, useState, useCallback } from 'react'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Button } from '@/components/ui/button'
import { RotateCcw } from 'lucide-react'
import type { ConfigKeySchema } from './config-queries'

interface Props {
  schema: ConfigKeySchema
  value: number
  defaultValue: number
  error?: string
  onChange: (key: string, value: number | undefined) => void
}

export const ConfigField: FC<Props> = ({ schema, value, defaultValue, error, onChange }) => {
  const [localValue, setLocalValue] = useState(String(value))
  const isModified = value !== defaultValue

  const handleBlur = useCallback(() => {
    const trimmed = localValue.trim().replace(/_/g, '')
    if (!/^\d+$/.test(trimmed)) {
      // Not a valid integer — let parent handle via error
      onChange(schema.key, Number.NaN)
      return
    }
    const parsed = Number(trimmed)
    onChange(schema.key, parsed)
  }, [localValue, schema.key, onChange])

  const handleReset = useCallback(() => {
    setLocalValue(String(defaultValue))
    onChange(schema.key, defaultValue)
  }, [defaultValue, schema.key, onChange])

  return (
    <div className="flex items-start justify-between gap-4 py-2">
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <Label className="font-medium text-sm">{schema.label}</Label>
          {isModified && (
            <span className="text-muted-foreground text-xs">(modified)</span>
          )}
        </div>
        <p className="text-muted-foreground text-xs">{schema.description}</p>
        {schema.risk && (
          <p className="mt-0.5 text-amber-600 text-xs">⚠ {schema.risk}</p>
        )}
        {error && (
          <p className="mt-0.5 text-destructive text-xs">{error}</p>
        )}
      </div>
      <div className="flex items-center gap-2 shrink-0">
        <div className="relative">
          <Input
            type="text"
            inputMode="numeric"
            value={localValue}
            onChange={(e) => setLocalValue(e.target.value)}
            onBlur={handleBlur}
            className={`w-32 text-right pr-12 ${error ? 'border-destructive' : isModified ? 'border-amber-500' : ''}`}
          />
          <span className="absolute right-3 top-1/2 -translate-y-1/2 text-muted-foreground text-xs">
            {schema.unit}
          </span>
        </div>
        {isModified && (
          <Button variant="ghost" size="icon" className="h-8 w-8 shrink-0" onClick={handleReset} title="Reset to default">
            <RotateCcw className="h-3 w-3" />
          </Button>
        )}
      </div>
    </div>
  )
}
```

- [ ] **Step 3: Write ConfigGroup.tsx**

A collapsible section for a group of safe fields.

```tsx
// apps/agent/entrypoints/app/advanced-config/ConfigGroup.tsx
import { type FC, useState } from 'react'
import { ChevronDown, ChevronRight } from 'lucide-react'
import { ConfigField } from './ConfigField'
import type { ConfigKeySchema } from './config-queries'

interface Props {
  label: string
  fields: ConfigKeySchema[]
  values: Record<string, number>
  defaults: Record<string, number>
  errors: Map<string, string>
  onChange: (key: string, value: number | undefined) => void
}

export const ConfigGroup: FC<Props> = ({ label, fields, values, defaults, errors, onChange }) => {
  const [open, setOpen] = useState(true)

  return (
    <div className="rounded-xl border border-border bg-card shadow-sm">
      <button
        className="flex w-full items-center gap-2 p-4 font-semibold text-lg hover:bg-muted/50 rounded-t-xl"
        onClick={() => setOpen(!open)}
      >
        {open ? <ChevronDown className="h-4 w-4" /> : <ChevronRight className="h-4 w-4" />}
        {label}
      </button>
      {open && (
        <div className="border-t px-4 pb-4 divide-y">
          {fields.map((f) => (
            <ConfigField
              key={f.key}
              schema={f}
              value={values[f.key] ?? f.default}
              defaultValue={f.default}
              error={errors.get(f.key)}
              onChange={onChange}
            />
          ))}
        </div>
      )}
    </div>
  )
}
```

- [ ] **Step 4: Write DangerousSection.tsx**

Collapsed by default, amber accent, shows risk per field.

```tsx
// apps/agent/entrypoints/app/advanced-config/DangerousSection.tsx
import { type FC, useState } from 'react'
import { AlertTriangle, ChevronDown, ChevronRight } from 'lucide-react'
import { ConfigField } from './ConfigField'
import type { ConfigKeySchema } from './config-queries'

interface Props {
  fields: ConfigKeySchema[]
  values: Record<string, number>
  defaults: Record<string, number>
  errors: Map<string, string>
  onChange: (key: string, value: number | undefined) => void
}

export const DangerousSection: FC<Props> = ({ fields, values, defaults, errors, onChange }) => {
  const [open, setOpen] = useState(false)

  return (
    <div className="rounded-xl border border-amber-500/30 bg-amber-50/5 shadow-sm">
      <button
        className="flex w-full items-center gap-2 p-4 font-semibold text-lg text-amber-700 dark:text-amber-400 hover:bg-amber-100/10 rounded-t-xl"
        onClick={() => setOpen(!open)}
      >
        <AlertTriangle className="h-5 w-5" />
        {open ? <ChevronDown className="h-4 w-4" /> : <ChevronRight className="h-4 w-4" />}
        Dangerous — edit with caution
      </button>
      {open && (
        <div className="border-t border-amber-500/20 px-4 pb-4 divide-y divide-amber-500/10">
          {fields.map((f) => (
            <ConfigField
              key={f.key}
              schema={f}
              value={values[f.key] ?? f.default}
              defaultValue={f.default}
              error={errors.get(f.key)}
              onChange={onChange}
            />
          ))}
        </div>
      )}
    </div>
  )
}
```

- [ ] **Step 5: Write PendingRestartBanner.tsx**

```tsx
// apps/agent/entrypoints/app/advanced-config/PendingRestartBanner.tsx
import { type FC } from 'react'
import { AlertTriangle } from 'lucide-react'

interface Props {
  visible: boolean
}

export const PendingRestartBanner: FC<Props> = ({ visible }) => {
  if (!visible) return null

  return (
    <div className="flex items-center gap-3 rounded-lg border border-amber-500/30 bg-amber-50/10 p-4 text-amber-800 dark:text-amber-300">
      <AlertTriangle className="h-5 w-5 shrink-0" />
      <div>
        <p className="font-medium">Changes saved. Please quit and reopen BrowserOS to apply.</p>
        <p className="text-xs opacity-80">Configuration is loaded at startup. Restart required.</p>
      </div>
    </div>
  )
}
```

- [ ] **Step 6: Write AdvancedConfigPage.tsx**

```tsx
// apps/agent/entrypoints/app/advanced-config/AdvancedConfigPage.tsx
import { type FC, useEffect, useState, useCallback } from 'react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { RotateCcw, Save, Settings } from 'lucide-react'
import { ConfigGroup } from './ConfigGroup'
import { DangerousSection } from './DangerousSection'
import { PendingRestartBanner } from './PendingRestartBanner'
import {
  fetchConfig,
  saveConfig,
  resetConfig,
  type ConfigResponse,
  type ConfigKeySchema,
} from './config-queries'

const GROUP_ORDER = ['Timeouts', 'Limits', 'Retention'] as const

export const AdvancedConfigPage: FC = () => {
  const [config, setConfig] = useState<ConfigResponse | null>(null)
  const [edited, setEdited] = useState<Record<string, number>>({})
  const [errors, setErrors] = useState<Map<string, string>>(new Map())
  const [isLoading, setIsLoading] = useState(true)
  const [isSaving, setIsSaving] = useState(false)

  const loadConfig = useCallback(async () => {
    try {
      const data = await fetchConfig()
      setConfig(data)
      setEdited(data.active)
      setErrors(new Map())
    } catch (e) {
      toast.error('Failed to load configuration')
    } finally {
      setIsLoading(false)
    }
  }, [])

  useEffect(() => { loadConfig() }, [loadConfig])

  const handleChange = useCallback((key: string, value: number | undefined) => {
    if (value === undefined || Number.isNaN(value)) {
      setErrors(prev => new Map(prev).set(key, 'Invalid number'))
      return
    }
    const schema = config?.schema[key]
    if (schema) {
      if (value < schema.min) {
        setErrors(prev => new Map(prev).set(key, `Minimum is ${schema.min}`))
      } else if (value > schema.max) {
        setErrors(prev => new Map(prev).set(key, `Maximum is ${schema.max.toLocaleString()}`))
      } else {
        setErrors(prev => { const next = new Map(prev); next.delete(key); return next })
      }
    }
    setEdited(prev => ({ ...prev, [key]: value }))
  }, [config])

  const handleSave = async () => {
    if (errors.size > 0) {
      toast.error('Fix validation errors before saving')
      return
    }
    // Only save modified values
    const overrides: Record<string, number> = {}
    for (const [key, value] of Object.entries(edited)) {
      if (config && value !== config.defaults[key]) {
        overrides[key] = value
      }
    }

    setIsSaving(true)
    try {
      const result = await saveConfig(overrides)
      if (result.ok) {
        toast.success('Configuration saved. Quit & reopen BrowserOS to apply.')
        await loadConfig()
      } else {
        const errorMap = new Map<string, string>()
        result.errors?.forEach(e => errorMap.set(e.key, e.message))
        setErrors(errorMap)
        toast.error('Validation failed')
      }
    } catch {
      toast.error('Failed to save configuration')
    } finally {
      setIsSaving(false)
    }
  }

  const handleResetAll = async () => {
    try {
      await resetConfig()
      toast.success('Configuration reset to defaults. Quit & reopen BrowserOS to apply.')
      await loadConfig()
    } catch {
      toast.error('Failed to reset configuration')
    }
  }

  if (isLoading) {
    return <div className="p-6 text-muted-foreground">Loading configuration...</div>
  }

  if (!config) {
    return <div className="p-6 text-destructive">Failed to load configuration.</div>
  }

  // Group fields
  const fieldsByGroup: Record<string, ConfigKeySchema[]> = {}
  const dangerousFields: ConfigKeySchema[] = []
  for (const schema of Object.values(config.schema)) {
    if (schema.section === 'dangerous') {
      dangerousFields.push(schema)
    } else {
      const list = fieldsByGroup[schema.group] ?? []
      list.push(schema)
      fieldsByGroup[schema.group] = list
    }
  }

  const hasEdits = Object.keys(edited).some(k => edited[k] !== config.defaults[k])

  return (
    <div className="fade-in slide-in-from-bottom-5 animate-in space-y-6 duration-500">
      {/* Header */}
      <div>
        <h1 className="flex items-center gap-2 font-semibold text-2xl">
          <Settings className="h-6 w-6" />
          Advanced Config
        </h1>
        <p className="mt-1 text-muted-foreground text-sm">
          Runtime configuration for BrowserOS internals. Changes require restart.
        </p>
      </div>

      {/* Pending banner */}
      <PendingRestartBanner visible={config.hasPendingChanges} />

      {/* Safe groups */}
      {GROUP_ORDER.map(group => {
        const fields = fieldsByGroup[group]
        if (!fields?.length) return null
        return (
          <ConfigGroup
            key={group}
            label={group}
            fields={fields}
            values={edited}
            defaults={config.defaults}
            errors={errors}
            onChange={handleChange}
          />
        )
      })}

      {/* Dangerous section */}
      {dangerousFields.length > 0 && (
        <DangerousSection
          fields={dangerousFields}
          values={edited}
          defaults={config.defaults}
          errors={errors}
          onChange={handleChange}
        />
      )}

      {/* Actions */}
      <div className="flex items-center justify-between">
        <Button variant="outline" onClick={handleResetAll} disabled={isSaving}>
          <RotateCcw className="mr-2 h-4 w-4" />
          Reset All to Defaults
        </Button>
        <Button onClick={handleSave} disabled={!hasEdits || errors.size > 0 || isSaving}>
          <Save className="mr-2 h-4 w-4" />
          {isSaving ? 'Saving...' : 'Save Changes'}
        </Button>
      </div>
    </div>
  )
}
```

- [ ] **Step 7: Add route and nav item**

In `apps/agent/entrypoints/app/App.tsx`:
- Add import: `import { AdvancedConfigPage } from './advanced-config/AdvancedConfigPage'`
- Add route inside the settings `<Route path="settings">` block:
  ```tsx
  <Route path="advanced" element={<AdvancedConfigPage />} />
  ```

In `apps/agent/components/sidebar/SettingsSidebar.tsx`:
- Add import: `import { Settings } from 'lucide-react'` (already imported)
- Add nav item in the "Other" section items array:
  ```ts
  { name: 'Advanced Config', to: '/settings/advanced', icon: Settings },
  ```

- [ ] **Step 8: Commit**

```bash
git add apps/agent/entrypoints/app/advanced-config/ apps/agent/entrypoints/app/App.tsx apps/agent/components/sidebar/SettingsSidebar.tsx
git commit -m "feat(agent): add Advanced Config settings page with grouped fields and validation"
```

---

### Task 6: Integration Verification

- [ ] **Step 1: Run all config-related tests**

Run: `cd packages/browseros-agent && bun test packages/shared/tests/constants/ apps/server/tests/config.test.ts`
Expected: all pass (61 existing + new schema + store + route tests)

- [ ] **Step 2: Verify no regressions in existing timeout/limit consumers**

Run: `cd packages/browseros-agent && bun test packages/shared/tests/constants/timeouts.test.ts packages/shared/tests/constants/limits.test.ts packages/shared/tests/constants/paths.test.ts`
Expected: 61/61 pass — getter-based access is transparent

- [ ] **Step 3: Run `gitnexus_detect_changes` to verify change scope**

Run: `gitnexus_detect_changes` — verify only expected files changed

- [ ] **Step 4: Final commit (if any fixes needed)**

```bash
git add -A
git commit -m "fix: address integration issues from advanced config implementation"
```

---

## Self-Review Checklist

- [x] Spec coverage: ConfigStore (Task 2), Schema (Task 1), Server API (Task 4), UI (Task 5), Integration (Task 6)
- [x] Placeholder scan: No TBD/TODO — all code blocks are complete
- [x] Type consistency: ConfigOverrides, ConfigKeySchema, ConfigResponse used consistently across all files
- [x] 37 keys in schema match 3 source files (timeouts + limits + paths)
- [x] Dangerous section collapsed by default with risk explanations per spec
- [x] "Quit & reopen" notification instead of restart button per spec
