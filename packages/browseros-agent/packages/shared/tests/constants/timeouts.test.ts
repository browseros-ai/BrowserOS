/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Tests for centralized timeout configuration.
 *
 * Env-override tests use Bun.spawn() to run a tiny inline script
 * in a child process — this guarantees a fresh module cache so that
 * process.env is read at import time with the overridden value.
 */

import path from 'node:path'
import { describe, expect, it } from 'bun:test'

// Absolute path to the module under test — computed once, baked into child code.
const TIMEOUTS_MODULE_PATH = JSON.stringify(
  path.resolve(import.meta.dir, '../../src/constants/timeouts.ts'),
)

// ---------------------------------------------------------------------------
// Helper: run a one-liner in a child Bun process with custom env
// ---------------------------------------------------------------------------

async function spawnWithEnv(envOverrides: Record<string, string>, code: string): Promise<string> {
  const proc = Bun.spawn(['bun', '-e', code], {
    env: { ...process.env, ...envOverrides, NO_COLOR: '1', FORCE_COLOR: '0' },
    stdout: 'pipe',
    stderr: 'pipe',
  })
  await proc.exited
  const stdout = await new Response(proc.stdout).text()
  if (proc.exitCode !== 0) {
    const stderr = await new Response(proc.stderr).text()
    throw new Error(`Child process exited ${proc.exitCode}: ${stderr}\n${stdout}`)
  }
  return stdout.trim()
}

// ---------------------------------------------------------------------------
// 1. Default values — all 26 constants
// ---------------------------------------------------------------------------

describe('TIMEOUTS default values', () => {
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const { TIMEOUTS } = require('../../src/constants/timeouts.ts') as { TIMEOUTS: Record<string, number> }

  it('has all expected timeout keys with correct default values', () => {
    // Agent/Tool execution
    expect(TIMEOUTS.TOOL_CALL).toBe(120_000)
    expect(TIMEOUTS.TOOL_POST_ACTION).toBe(2_000)
    expect(TIMEOUTS.TEST_PROVIDER).toBe(15_000)
    expect(TIMEOUTS.REFINE_PROMPT).toBe(30_000)

    // MCP operations
    expect(TIMEOUTS.MCP_DEFAULT).toBe(5_000)
    expect(TIMEOUTS.MCP_TRANSPORT_PROBE).toBe(5_000)
    expect(TIMEOUTS.MCP_CLIENT_CONNECT).toBe(15_000)

    // CDP connection
    expect(TIMEOUTS.CDP_CONNECT).toBe(10_000)
    expect(TIMEOUTS.CDP_CONNECT_RETRY_DELAY).toBe(1_000)
    expect(TIMEOUTS.CDP_RECONNECT_DELAY).toBe(5_000)
    expect(TIMEOUTS.CDP_KEEPALIVE_INTERVAL).toBe(30_000)
    expect(TIMEOUTS.CDP_KEEPALIVE_TIMEOUT).toBe(10_000)
    expect(TIMEOUTS.CDP_REQUEST_TIMEOUT).toBe(60_000)

    // External API calls
    expect(TIMEOUTS.KLAVIS_FETCH).toBe(30_000)
    expect(TIMEOUTS.SKILLS_FETCH).toBe(15_000)
    expect(TIMEOUTS.SKILLS_SYNC_INTERVAL).toBe(2_700_000) // 45 * 60_000

    // Navigation/DOM
    expect(TIMEOUTS.NAVIGATION).toBe(10_000)
    expect(TIMEOUTS.PAGE_LOAD_WAIT).toBe(30_000)
    expect(TIMEOUTS.PAGE_LOAD_POLL_INTERVAL).toBe(150)
    expect(TIMEOUTS.STABLE_DOM).toBe(3_000)
    expect(TIMEOUTS.FILE_CHOOSER).toBe(3_000)

    // OAuth
    expect(TIMEOUTS.OAUTH_FLOW_TTL).toBe(300_000)
    expect(TIMEOUTS.OAUTH_TOKEN_EXPIRY_BUFFER).toBe(300_000)
    expect(TIMEOUTS.OAUTH_POLL_INTERVAL).toBe(2_000)
    expect(TIMEOUTS.OAUTH_POLL_TIMEOUT).toBe(300_000)
    expect(TIMEOUTS.DEVICE_CODE_POLL_SAFETY_MARGIN).toBe(3_000)
  })

  it('exports exactly 26 timeout keys', () => {
    expect(Object.keys(TIMEOUTS)).toHaveLength(26)
  })

  it('all values are positive numbers', () => {
    for (const [key, value] of Object.entries(TIMEOUTS)) {
      expect(typeof value).toBe('number')
      expect(value).toBeGreaterThan(0)
      expect(Number.isFinite(value)).toBe(true)
    }
  })
})

// ---------------------------------------------------------------------------
// 2. Env override tests — subprocess isolation
// ---------------------------------------------------------------------------

describe('TIMEOUTS env override', () => {
  it('overrides TOOL_CALL via BROWSEROS_TIMEOUT_TOOL_CALL', async () => {
    const result = await spawnWithEnv(
      { BROWSEROS_TIMEOUT_TOOL_CALL: '99999' },
      `const { TIMEOUTS } = require(${TIMEOUTS_MODULE_PATH}); console.log(TIMEOUTS.TOOL_CALL)`,
    )
    expect(Number(result)).toBe(99999)
  })

  it('overrides MCP_CLIENT_CONNECT via BROWSEROS_TIMEOUT_MCP_CLIENT_CONNECT', async () => {
    const result = await spawnWithEnv(
      { BROWSEROS_TIMEOUT_MCP_CLIENT_CONNECT: '25000' },
      `const { TIMEOUTS } = require(${TIMEOUTS_MODULE_PATH}); console.log(TIMEOUTS.MCP_CLIENT_CONNECT)`,
    )
    expect(Number(result)).toBe(25_000)
  })

  it('overrides CDP_REQUEST_TIMEOUT via BROWSEROS_TIMEOUT_CDP_REQUEST_TIMEOUT', async () => {
    const result = await spawnWithEnv(
      { BROWSEROS_TIMEOUT_CDP_REQUEST_TIMEOUT: '90000' },
      `const { TIMEOUTS } = require(${TIMEOUTS_MODULE_PATH}); console.log(TIMEOUTS.CDP_REQUEST_TIMEOUT)`,
    )
    expect(Number(result)).toBe(90_000)
  })

  it('overrides SKILLS_SYNC_INTERVAL via BROWSEROS_TIMEOUT_SKILLS_SYNC_INTERVAL', async () => {
    const result = await spawnWithEnv(
      { BROWSEROS_TIMEOUT_SKILLS_SYNC_INTERVAL: '60000' },
      `const { TIMEOUTS } = require(${TIMEOUTS_MODULE_PATH}); console.log(TIMEOUTS.SKILLS_SYNC_INTERVAL)`,
    )
    expect(Number(result)).toBe(60_000)
  })

  it('overrides OAUTH_FLOW_TTL via BROWSEROS_TIMEOUT_OAUTH_FLOW_TTL', async () => {
    const result = await spawnWithEnv(
      { BROWSEROS_TIMEOUT_OAUTH_FLOW_TTL: '120000' },
      `const { TIMEOUTS } = require(${TIMEOUTS_MODULE_PATH}); console.log(TIMEOUTS.OAUTH_FLOW_TTL)`,
    )
    expect(Number(result)).toBe(120_000)
  })
})

// ---------------------------------------------------------------------------
// 3. Invalid env — non-numeric
// ---------------------------------------------------------------------------

describe('TIMEOUTS invalid env fallback', () => {
  it('falls back to default when env is non-numeric', async () => {
    const result = await spawnWithEnv(
      { BROWSEROS_TIMEOUT_TOOL_CALL: 'abc' },
      `const { TIMEOUTS } = require(${TIMEOUTS_MODULE_PATH}); console.log(TIMEOUTS.TOOL_CALL)`,
    )
    expect(Number(result)).toBe(120_000)
  })

  it('falls back to default when env has trailing chars', async () => {
    const result = await spawnWithEnv(
      { BROWSEROS_TIMEOUT_TOOL_CALL: '10s' },
      `const { TIMEOUTS } = require(${TIMEOUTS_MODULE_PATH}); console.log(TIMEOUTS.TOOL_CALL)`,
    )
    expect(Number(result)).toBe(120_000)
  })
})

// ---------------------------------------------------------------------------
// 4. Negative env
// ---------------------------------------------------------------------------

describe('TIMEOUTS negative env fallback', () => {
  it('falls back to default when env is negative', async () => {
    const result = await spawnWithEnv(
      { BROWSEROS_TIMEOUT_TOOL_CALL: '-1' },
      `const { TIMEOUTS } = require(${TIMEOUTS_MODULE_PATH}); console.log(TIMEOUTS.TOOL_CALL)`,
    )
    expect(Number(result)).toBe(120_000)
  })
})

// ---------------------------------------------------------------------------
// 5. KLAVIS_PROXY_RETRY_BACKOFF_MS — hardcoded array, never affected by env
// ---------------------------------------------------------------------------

describe('KLAVIS_PROXY_RETRY_BACKOFF_MS', () => {
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const { KLAVIS_PROXY_RETRY_BACKOFF_MS } = require('../../src/constants/timeouts.ts') as { KLAVIS_PROXY_RETRY_BACKOFF_MS: readonly number[] }

  it('exports the correct retry backoff array', () => {
    expect(KLAVIS_PROXY_RETRY_BACKOFF_MS).toEqual([5_000, 10_000, 20_000, 40_000, 60_000])
  })

  it('is a tuple of 5 elements', () => {
    expect(KLAVIS_PROXY_RETRY_BACKOFF_MS).toHaveLength(5)
  })

  it('values are in ascending order', () => {
    for (let i = 1; i < KLAVIS_PROXY_RETRY_BACKOFF_MS.length; i++) {
      expect(KLAVIS_PROXY_RETRY_BACKOFF_MS[i]).toBeGreaterThan(KLAVIS_PROXY_RETRY_BACKOFF_MS[i - 1])
    }
  })
})
