/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, test } from 'bun:test'
import {
  CONFIG_KEY_MAP,
  CONFIG_KEYS,
  ConfigOverridesSchema,
  validateAllOverrides,
  validateConfigValue,
} from '../../src/constants/config-schema'

describe('CONFIG_KEYS', () => {
  test('has 37 keys total', () => {
    expect(CONFIG_KEYS).toHaveLength(37)
  })

  test('all keys have unique dot-paths', () => {
    const keys = CONFIG_KEYS.map((configKey) => configKey.key)
    expect(new Set(keys).size).toBe(keys.length)
  })

  test('all keys have required fields', () => {
    for (const configKey of CONFIG_KEYS) {
      expect(configKey.key).toBeTruthy()
      expect(configKey.label).toBeTruthy()
      expect(configKey.group).toMatch(/^(Timeouts|Limits|Retention)$/)
      expect(configKey.section).toMatch(/^(safe|dangerous)$/)
      expect(configKey.unit).toBeTruthy()
      expect(configKey.min).toBeGreaterThanOrEqual(0)
      expect(configKey.max).toBeGreaterThan(configKey.min)
      expect(configKey.default).toBeGreaterThanOrEqual(configKey.min)
      expect(configKey.default).toBeLessThanOrEqual(configKey.max)
      expect(configKey.description).toBeTruthy()
      expect(configKey.envVar).toBeTruthy()
    }
  })

  test('dangerous keys have risk explanations', () => {
    const dangerousKeys = CONFIG_KEYS.filter(
      (configKey) => configKey.section === 'dangerous',
    )
    expect(dangerousKeys.length).toBeGreaterThan(0)

    for (const configKey of dangerousKeys) {
      expect(configKey.risk).toBeTruthy()
    }
  })

  test('CONFIG_KEY_MAP matches CONFIG_KEYS', () => {
    expect(CONFIG_KEY_MAP.size).toBe(CONFIG_KEYS.length)

    for (const configKey of CONFIG_KEYS) {
      expect(CONFIG_KEY_MAP.get(configKey.key)).toBe(configKey)
    }
  })
})

describe('validateConfigValue', () => {
  test('accepts valid value within range', () => {
    expect(validateConfigValue('TIMEOUTS.TOOL_CALL', 60_000)).toBeNull()
  })

  test('rejects unknown key', () => {
    expect(validateConfigValue('UNKNOWN.KEY', 100)).toContain(
      'Unknown config key',
    )
  })

  test('rejects value below minimum', () => {
    expect(validateConfigValue('TIMEOUTS.TOOL_CALL', 500)).toContain(
      'below minimum',
    )
  })

  test('rejects value above maximum for bounded keys', () => {
    expect(validateConfigValue('TIMEOUTS.TEST_PROVIDER', 999_999)).toContain(
      'exceeds maximum',
    )
  })

  test('accepts very large values for unbounded keys', () => {
    expect(validateConfigValue('AGENT_LIMITS.MAX_TURNS', 99_999)).toBeNull()
    expect(validateConfigValue('TIMEOUTS.TOOL_CALL', 7_200_000)).toBeNull()
    expect(
      validateConfigValue('TOOL_LIMITS.FILESYSTEM_READ_MAX_CHARS', 1_500_000),
    ).toBeNull()
  })

  test('rejects negative value', () => {
    expect(validateConfigValue('TIMEOUTS.TOOL_CALL', -1)).toContain(
      'non-negative',
    )
  })

  test('accepts exact min boundary', () => {
    expect(validateConfigValue('TIMEOUTS.TOOL_CALL', 1_000)).toBeNull()
  })

  test('accepts exact max boundary for bounded key', () => {
    expect(validateConfigValue('TIMEOUTS.TEST_PROVIDER', 120_000)).toBeNull()
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
      'AGENT_LIMITS.MAX_TURNS': 9_999,
      'UNKNOWN.KEY': 100,
    })

    // MAX_TURNS is now unbounded so 9999 is valid; only -1 and UNKNOWN.KEY are errors
    expect(errors.size).toBe(2)
  })
})

describe('ConfigOverridesSchema', () => {
  test('accepts valid overrides', () => {
    const result = ConfigOverridesSchema.safeParse({
      'TIMEOUTS.TOOL_CALL': 60_000,
    })
    expect(result.success).toBe(true)
  })

  test('rejects non-integer values', () => {
    const result = ConfigOverridesSchema.safeParse({
      'TIMEOUTS.TOOL_CALL': 1.5,
    })
    expect(result.success).toBe(false)
  })

  test('rejects string values', () => {
    const result = ConfigOverridesSchema.safeParse({
      'TIMEOUTS.TOOL_CALL': '60000',
    })
    expect(result.success).toBe(false)
  })
})
