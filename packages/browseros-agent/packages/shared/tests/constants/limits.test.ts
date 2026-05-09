import path from 'node:path'
/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Tests for configurable limits constants.
 *
 * Env-override tests use Bun.spawn() to run a tiny inline script
 * in a child process — this guarantees a fresh module cache so that
 * process.env is read at import time with the overridden value.
 */

import { describe, expect, it } from 'bun:test'

// ---------------------------------------------------------------------------
// Helper: run a one-liner in a child Bun process with custom env
// ---------------------------------------------------------------------------

async function spawnWithEnv(envOverrides: Record<string, string>, code: string): Promise<string> {
  const proc = Bun.spawn(['bun', '-e', code], {
    env: { ...process.env, ...envOverrides, NO_COLOR: '1', FORCE_COLOR: '0' } as Record<string, string>,
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

const modDir = import.meta.dir

// ---------------------------------------------------------------------------
// Default values — imported once for synchronous checks
// ---------------------------------------------------------------------------

describe('AGENT_LIMITS defaults', () => {
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const { AGENT_LIMITS } = require('../../src/constants/limits.ts') as typeof import('../../src/constants/limits')

  it('has correct default MAX_TURNS', () => {
    expect(AGENT_LIMITS.MAX_TURNS).toBe(100)
  })

  it('has correct default DEFAULT_CONTEXT_WINDOW', () => {
    expect(AGENT_LIMITS.DEFAULT_CONTEXT_WINDOW).toBe(200_000)
  })

  it('has correct default COMPACTION_SUMMARIZATION_TIMEOUT_MS', () => {
    expect(AGENT_LIMITS.COMPACTION_SUMMARIZATION_TIMEOUT_MS).toBe(60_000)
  })
})

describe('TOOL_LIMITS defaults', () => {
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const { TOOL_LIMITS } = require('../../src/constants/limits.ts') as typeof import('../../src/constants/limits')

  it('has correct default FILESYSTEM_READ_MAX_LINES', () => {
    expect(TOOL_LIMITS.FILESYSTEM_READ_MAX_LINES).toBe(500)
  })

  it('has correct default FILESYSTEM_READ_MAX_CHARS', () => {
    expect(TOOL_LIMITS.FILESYSTEM_READ_MAX_CHARS).toBe(15_000)
  })

  it('has correct default INLINE_PAGE_CONTENT_MAX_CHARS', () => {
    expect(TOOL_LIMITS.INLINE_PAGE_CONTENT_MAX_CHARS).toBe(5_000)
  })
})

describe('AGENT_LIMITS non-overridable defaults', () => {
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const { AGENT_LIMITS } = require('../../src/constants/limits.ts') as typeof import('../../src/constants/limits')

  it('has correct COMPRESSION_MIN_HEADROOM', () => {
    expect(AGENT_LIMITS.COMPRESSION_MIN_HEADROOM).toBe(10_000)
  })

  it('has correct COMPACTION_RESERVE_TOKENS', () => {
    expect(AGENT_LIMITS.COMPACTION_RESERVE_TOKENS).toBe(20_000)
  })

  it('has correct COMPACTION_MAX_SUMMARIZATION_INPUT', () => {
    expect(AGENT_LIMITS.COMPACTION_MAX_SUMMARIZATION_INPUT).toBe(100_000)
  })

  it('has correct COMPACTION_TOOL_OUTPUT_MAX_CHARS', () => {
    expect(AGENT_LIMITS.COMPACTION_TOOL_OUTPUT_MAX_CHARS).toBe(15_000)
  })

  it('has correct COMPACTION_FIXED_OVERHEAD', () => {
    expect(AGENT_LIMITS.COMPACTION_FIXED_OVERHEAD).toBe(12_000)
  })
})

describe('PAGINATION defaults', () => {
  it('has correct DEFAULT_PAGE_SIZE', () => {
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const { PAGINATION } = require('../../src/constants/limits.ts')
    expect(PAGINATION.DEFAULT_PAGE_SIZE).toBe(20)
  })
})

describe('CDP_LIMITS defaults', () => {
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const { CDP_LIMITS } = require('../../src/constants/limits.ts')

  it('has correct CONNECT_MAX_RETRIES', () => {
    expect(CDP_LIMITS.CONNECT_MAX_RETRIES).toBe(3)
  })

  it('has correct RECONNECT_MAX_RETRIES', () => {
    expect(CDP_LIMITS.RECONNECT_MAX_RETRIES).toBe(3)
  })
})

describe('CONTENT_LIMITS defaults', () => {
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const { CONTENT_LIMITS } = require('../../src/constants/limits.ts')

  it('has correct BODY_CONTEXT_SIZE', () => {
    expect(CONTENT_LIMITS.BODY_CONTEXT_SIZE).toBe(10_000)
  })

  it('has correct MAX_QUEUE_SIZE', () => {
    expect(CONTENT_LIMITS.MAX_QUEUE_SIZE).toBe(1_000)
  })
})

describe('AGENT_HARNESS_LIMITS defaults', () => {
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const { AGENT_HARNESS_LIMITS } = require('../../src/constants/limits.ts')

  it('has correct AGENT_NAME_MAX_CHARS', () => {
    expect(AGENT_HARNESS_LIMITS.AGENT_NAME_MAX_CHARS).toBe(80)
  })

  it('has correct QUEUE_MAX_LENGTH', () => {
    expect(AGENT_HARNESS_LIMITS.QUEUE_MAX_LENGTH).toBe(50)
  })

  it('has correct QUEUE_MESSAGE_MAX_BYTES', () => {
    expect(AGENT_HARNESS_LIMITS.QUEUE_MESSAGE_MAX_BYTES).toBe(64 * 1024)
  })
})

// ---------------------------------------------------------------------------
// ENV OVERRIDE TESTS — subprocess isolation
// ---------------------------------------------------------------------------

describe('AGENT_LIMITS env override', () => {
  it('overrides MAX_TURNS via BROWSEROS_LIMIT_MAX_TURNS', async () => {
    const result = await spawnWithEnv(
      { BROWSEROS_LIMIT_MAX_TURNS: '200' },
      `const { AGENT_LIMITS } = require('/home/bhd/Documents/Projects/bhd/BrowserOS/packages/browseros-agent/packages/shared/src/constants/limits.ts'); console.log(AGENT_LIMITS.MAX_TURNS)`,
    )
    expect(Number(result)).toBe(200)
  })

  it('overrides DEFAULT_CONTEXT_WINDOW via BROWSEROS_LIMIT_DEFAULT_CONTEXT_WINDOW', async () => {
    const result = await spawnWithEnv(
      { BROWSEROS_LIMIT_DEFAULT_CONTEXT_WINDOW: '500000' },
      `const { AGENT_LIMITS } = require('/home/bhd/Documents/Projects/bhd/BrowserOS/packages/browseros-agent/packages/shared/src/constants/limits.ts'); console.log(AGENT_LIMITS.DEFAULT_CONTEXT_WINDOW)`,
    )
    expect(Number(result)).toBe(500000)
  })

  it('overrides FILESYSTEM_READ_MAX_LINES via BROWSEROS_LIMIT_FILESYSTEM_READ_MAX_LINES', async () => {
    const result = await spawnWithEnv(
      { BROWSEROS_LIMIT_FILESYSTEM_READ_MAX_LINES: '1000' },
      `const { TOOL_LIMITS } = require('/home/bhd/Documents/Projects/bhd/BrowserOS/packages/browseros-agent/packages/shared/src/constants/limits.ts'); console.log(TOOL_LIMITS.FILESYSTEM_READ_MAX_LINES)`,
    )
    expect(Number(result)).toBe(1000)
  })
})

describe('AGENT_LIMITS invalid env fallback', () => {
  it('falls back to default when BROWSEROS_LIMIT_MAX_TURNS is non-numeric', async () => {
    const result = await spawnWithEnv(
      { BROWSEROS_LIMIT_MAX_TURNS: 'abc' },
      `const { AGENT_LIMITS } = require('/home/bhd/Documents/Projects/bhd/BrowserOS/packages/browseros-agent/packages/shared/src/constants/limits.ts'); console.log(AGENT_LIMITS.MAX_TURNS)`,
    )
    expect(Number(result)).toBe(100)
  })
})

describe('AGENT_LIMITS negative env fallback', () => {
  it('falls back to default when BROWSEROS_LIMIT_MAX_TURNS is negative', async () => {
    const result = await spawnWithEnv(
      { BROWSEROS_LIMIT_MAX_TURNS: '-1' },
      `const { AGENT_LIMITS } = require('/home/bhd/Documents/Projects/bhd/BrowserOS/packages/browseros-agent/packages/shared/src/constants/limits.ts'); console.log(AGENT_LIMITS.MAX_TURNS)`,
    )
    expect(Number(result)).toBe(100)
  })
})
