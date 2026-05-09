import path from 'node:path'
/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Tests for configurable path constants.
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
// Default values — retention and sizing
// ---------------------------------------------------------------------------

describe('PATHS defaults — retention and sizing', () => {
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const { PATHS } = require('../../src/constants/paths.ts') as typeof import('../../src/constants/paths')

  it('has correct default SOUL_MAX_LINES', () => {
    expect(PATHS.SOUL_MAX_LINES).toBe(150)
  })

  it('has correct default MEMORY_RETENTION_DAYS', () => {
    expect(PATHS.MEMORY_RETENTION_DAYS).toBe(30)
  })

  it('has correct default SESSION_RETENTION_DAYS', () => {
    expect(PATHS.SESSION_RETENTION_DAYS).toBe(30)
  })
})

describe('PATHS defaults — directory names', () => {
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const { PATHS } = require('../../src/constants/paths.ts') as typeof import('../../src/constants/paths')

  it('has correct BROWSEROS_DIR_NAME', () => expect(PATHS.BROWSEROS_DIR_NAME).toBe('.browseros'))
  it('has correct DEV_BROWSEROS_DIR_NAME', () => expect(PATHS.DEV_BROWSEROS_DIR_NAME).toBe('.browseros-dev'))
  it('has correct CACHE_DIR_NAME', () => expect(PATHS.CACHE_DIR_NAME).toBe('cache'))
  it('has correct DB_DIR_NAME', () => expect(PATHS.DB_DIR_NAME).toBe('db'))
  it('has correct DB_FILE_NAME', () => expect(PATHS.DB_FILE_NAME).toBe('browseros.sqlite'))
  it('has correct MEMORY_DIR_NAME', () => expect(PATHS.MEMORY_DIR_NAME).toBe('memory'))
  it('has correct SESSIONS_DIR_NAME', () => expect(PATHS.SESSIONS_DIR_NAME).toBe('sessions'))
  it('has correct SKILLS_DIR_NAME', () => expect(PATHS.SKILLS_DIR_NAME).toBe('skills'))
  it('has correct BUILTIN_DIR_NAME', () => expect(PATHS.BUILTIN_DIR_NAME).toBe('builtin'))
  it('has correct SERVER_CONFIG_FILE_NAME', () => expect(PATHS.SERVER_CONFIG_FILE_NAME).toBe('server.json'))
  it('has correct SOUL_FILE_NAME', () => expect(PATHS.SOUL_FILE_NAME).toBe('SOUL.md'))
  it('has correct CORE_MEMORY_FILE_NAME', () => expect(PATHS.CORE_MEMORY_FILE_NAME).toBe('CORE.md'))
})

describe('PATHS defaults — dynamic values', () => {
  it('DEFAULT_EXECUTION_DIR equals process.cwd()', () => {
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const { PATHS } = require('../../src/constants/paths.ts')
    expect(PATHS.DEFAULT_EXECUTION_DIR).toBe(process.cwd())
  })
})

// ---------------------------------------------------------------------------
// ENV OVERRIDE TESTS — subprocess isolation
// ---------------------------------------------------------------------------

describe('PATHS env override', () => {
  it('overrides SOUL_MAX_LINES via BROWSEROS_LIMIT_SOUL_MAX_LINES', async () => {
    const result = await spawnWithEnv(
      { BROWSEROS_LIMIT_SOUL_MAX_LINES: '300' },
      `const { PATHS } = require('/home/bhd/Documents/Projects/bhd/BrowserOS/packages/browseros-agent/packages/shared/src/constants/paths.ts'); console.log(PATHS.SOUL_MAX_LINES)`,
    )
    expect(Number(result)).toBe(300)
  })

  it('overrides MEMORY_RETENTION_DAYS via BROWSEROS_LIMIT_MEMORY_RETENTION_DAYS', async () => {
    const result = await spawnWithEnv(
      { BROWSEROS_LIMIT_MEMORY_RETENTION_DAYS: '90' },
      `const { PATHS } = require('/home/bhd/Documents/Projects/bhd/BrowserOS/packages/browseros-agent/packages/shared/src/constants/paths.ts'); console.log(PATHS.MEMORY_RETENTION_DAYS)`,
    )
    expect(Number(result)).toBe(90)
  })

  it('overrides SESSION_RETENTION_DAYS via BROWSEROS_LIMIT_SESSION_RETENTION_DAYS', async () => {
    const result = await spawnWithEnv(
      { BROWSEROS_LIMIT_SESSION_RETENTION_DAYS: '60' },
      `const { PATHS } = require('/home/bhd/Documents/Projects/bhd/BrowserOS/packages/browseros-agent/packages/shared/src/constants/paths.ts'); console.log(PATHS.SESSION_RETENTION_DAYS)`,
    )
    expect(Number(result)).toBe(60)
  })
})

describe('PATHS invalid env fallback', () => {
  it('falls back to default when BROWSEROS_LIMIT_SOUL_MAX_LINES is non-numeric', async () => {
    const result = await spawnWithEnv(
      { BROWSEROS_LIMIT_SOUL_MAX_LINES: 'abc' },
      `const { PATHS } = require('/home/bhd/Documents/Projects/bhd/BrowserOS/packages/browseros-agent/packages/shared/src/constants/paths.ts'); console.log(PATHS.SOUL_MAX_LINES)`,
    )
    expect(Number(result)).toBe(150)
  })
})

describe('PATHS negative env fallback', () => {
  it('falls back to default when BROWSEROS_LIMIT_SOUL_MAX_LINES is negative', async () => {
    const result = await spawnWithEnv(
      { BROWSEROS_LIMIT_SOUL_MAX_LINES: '-1' },
      `const { PATHS } = require('/home/bhd/Documents/Projects/bhd/BrowserOS/packages/browseros-agent/packages/shared/src/constants/paths.ts'); console.log(PATHS.SOUL_MAX_LINES)`,
    )
    expect(Number(result)).toBe(150)
  })
})
