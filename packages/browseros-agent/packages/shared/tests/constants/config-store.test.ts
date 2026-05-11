/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { existsSync, mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { ConfigStore } from '../../src/constants/config-store'

let testDir: string
let configFilePath: string
let store: ConfigStore

function clearEnv(): void {
  delete process.env.BROWSEROS_TIMEOUT_TOOL_CALL
  delete process.env.BROWSEROS_LIMIT_MAX_TURNS
}

beforeEach(() => {
  testDir = join(
    tmpdir(),
    `browseros-config-store-${Date.now()}-${Math.random().toString(16).slice(2)}`,
  )
  mkdirSync(testDir, { recursive: true })
  configFilePath = join(testDir, 'advanced-config.json')
  store = new ConfigStore()
  clearEnv()
})

afterEach(() => {
  clearEnv()
  rmSync(testDir, { recursive: true, force: true })
})

describe('ConfigStore defaults', () => {
  test('returns defaults before init is called', () => {
    expect(store.get('TIMEOUTS.TOOL_CALL')).toBe(120_000)
    expect(store.get('AGENT_LIMITS.MAX_TURNS')).toBe(100)
    expect(store.get('PATHS.SESSION_RETENTION_DAYS')).toBe(30)
  })

  test('throws on unknown keys', () => {
    expect(() => store.get('UNKNOWN.KEY')).toThrow(
      'Unknown config key: UNKNOWN.KEY',
    )
  })

  test('getAllActive returns every configured key', () => {
    const active = store.getAllActive()
    expect(Object.keys(active)).toHaveLength(37)
    expect(active['TIMEOUTS.TOOL_CALL']).toBe(120_000)
  })

  test('getDefaults returns default values for every configured key', () => {
    const defaults = store.getDefaults()
    expect(Object.keys(defaults)).toHaveLength(37)
    expect(defaults['AGENT_LIMITS.DEFAULT_CONTEXT_WINDOW']).toBe(200_000)
  })
})

describe('ConfigStore file overrides', () => {
  test('reads valid overrides from disk on init', () => {
    writeFileSync(
      configFilePath,
      `${JSON.stringify({ 'TIMEOUTS.TOOL_CALL': 60_000 })}\n`,
    )
    store.init(configFilePath)

    expect(store.get('TIMEOUTS.TOOL_CALL')).toBe(60_000)
    expect(store.get('AGENT_LIMITS.MAX_TURNS')).toBe(100)
  })

  test('ignores corrupt JSON files', () => {
    writeFileSync(configFilePath, '{not-json')
    store.init(configFilePath)

    expect(store.get('TIMEOUTS.TOOL_CALL')).toBe(120_000)
  })

  test('ignores non-object JSON files', () => {
    writeFileSync(configFilePath, '123')
    store.init(configFilePath)

    expect(store.get('TIMEOUTS.TOOL_CALL')).toBe(120_000)
  })
})

describe('ConfigStore env precedence', () => {
  test('env wins over file overrides', () => {
    writeFileSync(
      configFilePath,
      `${JSON.stringify({ 'TIMEOUTS.TOOL_CALL': 60_000 })}\n`,
    )
    process.env.BROWSEROS_TIMEOUT_TOOL_CALL = '90_000'
    store.init(configFilePath)

    expect(store.get('TIMEOUTS.TOOL_CALL')).toBe(90_000)
  })

  test('invalid env falls back to file override', () => {
    writeFileSync(
      configFilePath,
      `${JSON.stringify({ 'TIMEOUTS.TOOL_CALL': 60_000 })}\n`,
    )
    process.env.BROWSEROS_TIMEOUT_TOOL_CALL = 'oops'
    store.init(configFilePath)

    expect(store.get('TIMEOUTS.TOOL_CALL')).toBe(60_000)
  })

  test('invalid env falls back to defaults when file missing', () => {
    process.env.BROWSEROS_LIMIT_MAX_TURNS = '-1'

    expect(store.get('AGENT_LIMITS.MAX_TURNS')).toBe(100)
  })
})

describe('ConfigStore save and reset', () => {
  test('save validates and persists overrides without changing active values until re-init', () => {
    store.init(configFilePath)

    const errors = store.save({
      'TIMEOUTS.TOOL_CALL': 45_000,
      'AGENT_LIMITS.MAX_TURNS': 42,
    })

    expect(errors.size).toBe(0)
    expect(store.get('TIMEOUTS.TOOL_CALL')).toBe(120_000)
    expect(store.get('AGENT_LIMITS.MAX_TURNS')).toBe(100)
    expect(store.getFileOverrides()).toEqual({
      'TIMEOUTS.TOOL_CALL': 45_000,
      'AGENT_LIMITS.MAX_TURNS': 42,
    })
    expect(existsSync(configFilePath)).toBe(true)
  })

  test('save rejects invalid overrides without mutating state', () => {
    store.init(configFilePath)

    const errors = store.save({ 'TIMEOUTS.TOOL_CALL': -1 })

    expect(errors.get('TIMEOUTS.TOOL_CALL')).toContain('non-negative')
    expect(store.get('TIMEOUTS.TOOL_CALL')).toBe(120_000)
    expect(existsSync(configFilePath)).toBe(false)
  })

  test('save rejects unknown keys', () => {
    store.init(configFilePath)

    const errors = store.save({ 'UNKNOWN.KEY': 100 })

    expect(errors.get('UNKNOWN.KEY')).toBe('Unknown config key: UNKNOWN.KEY')
  })

  test('getFileOverrides returns the pending file layer only', () => {
    store.init(configFilePath)
    store.save({ 'TIMEOUTS.TOOL_CALL': 45_000 })

    expect(store.getFileOverrides()).toEqual({ 'TIMEOUTS.TOOL_CALL': 45_000 })
  })

  test('reset clears active, pending, and on-disk overrides', () => {
    store.init(configFilePath)
    store.save({ 'TIMEOUTS.TOOL_CALL': 45_000 })

    store.reset()

    expect(store.get('TIMEOUTS.TOOL_CALL')).toBe(120_000)
    expect(store.getFileOverrides()).toEqual({})
    expect(existsSync(configFilePath)).toBe(false)
  })
})

describe('ConfigStore pending changes', () => {
  test('returns false when no saved overrides exist', () => {
    store.init(configFilePath)

    expect(store.hasPendingChanges()).toBe(false)
  })

  test('returns true when pending overrides differ from active values', () => {
    store.init(configFilePath)
    store.save({ 'TIMEOUTS.TOOL_CALL': 45_000 })

    expect(store.hasPendingChanges()).toBe(true)
  })

  test('returns false after restart when saved overrides become active', () => {
    store.init(configFilePath)
    store.save({ 'TIMEOUTS.TOOL_CALL': 45_000 })
    store.init(configFilePath)

    expect(store.get('TIMEOUTS.TOOL_CALL')).toBe(45_000)
    expect(store.hasPendingChanges()).toBe(false)
  })
})

describe('ConfigStore ENV clamping', () => {
  test('clamps ENV value below min to min', () => {
    //#given
    process.env.BROWSEROS_LIMIT_MAX_TURNS = '0'

    //#when
    const value = store.get('AGENT_LIMITS.MAX_TURNS')

    //#then
    expect(value).toBe(1)
  })

  test('allows ENV value at min boundary', () => {
    //#given
    process.env.BROWSEROS_LIMIT_MAX_TURNS = '1'

    //#when
    const value = store.get('AGENT_LIMITS.MAX_TURNS')

    //#then
    expect(value).toBe(1)
  })

  test('allows ENV value well above min', () => {
    //#given
    process.env.BROWSEROS_LIMIT_MAX_TURNS = '9999'

    //#when
    const value = store.get('AGENT_LIMITS.MAX_TURNS')

    //#then
    expect(value).toBe(9999)
  })
})
