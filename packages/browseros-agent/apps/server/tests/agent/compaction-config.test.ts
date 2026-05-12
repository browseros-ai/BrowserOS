/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Tests for CompactionStrategyConfig type and config wiring.
 *
 * Testing order (worst-first):
 * - Zone 4: Invalid method → error propagation
 * - Zone 3: Empty/nil config
 * - Zone 3: Multi-flag invalid
 * - Zone 2: Boundary values
 * - Zone 1: Happy path
 */
import { describe, expect, it } from 'bun:test'
import { resolveCompactionConfig } from '../../src/agent/compaction-config'
import type { CompactionStrategyConfig } from '../../src/agent/types'

// ─── Zone 4: Error propagation ──────────────────────────────

describe('CompactionStrategyConfig — method defaults', () => {
  it('invalid method value → defaults to "default"', () => {
    const raw = { method: 'invalid' }
    const result = resolveCompactionConfig(raw)
    expect(result).toBeDefined()
    expect(result!.method).toBe('default')
  })

  it('method being a number → defaults to "default"', () => {
    const raw = { method: 42 }
    const result = resolveCompactionConfig(raw)
    expect(result).toBeDefined()
    expect(result!.method).toBe('default')
  })

  it('method being null → defaults to "default"', () => {
    const raw = { method: null }
    const result = resolveCompactionConfig(raw)
    expect(result).toBeDefined()
    expect(result!.method).toBe('default')
  })
})

// ─── Zone 3: Empty / nil / multi-flag ───────────────────────

describe('CompactionStrategyConfig — empty/nil', () => {
  it('returns undefined when no compaction config provided', () => {
    const result = resolveCompactionConfig(undefined)
    expect(result).toBeUndefined()
  })

  it('returns undefined for empty object', () => {
    const result = resolveCompactionConfig({})
    expect(result).toBeUndefined()
  })
})

describe('CompactionStrategyConfig — multi-flag invalid', () => {
  it('throws when method=vcc but vccConfig has negative maxTranscriptLines', () => {
    const raw = {
      method: 'vcc',
      vccConfig: { maxTranscriptLines: -1 },
    }
    expect(() => resolveCompactionConfig(raw)).toThrow()
  })

  it('throws when method=vcc and vccConfig has negative maxGoalLines', () => {
    const raw = {
      method: 'vcc',
      vccConfig: { maxGoalLines: -5 },
    }
    expect(() => resolveCompactionConfig(raw)).toThrow()
  })

  it('throws when method=default but customPrompt is empty string', () => {
    const raw = {
      method: 'default',
      customPrompt: '',
    }
    expect(() => resolveCompactionConfig(raw)).toThrow(/empty|blank/i)
  })
})

// ─── Zone 2: Boundary values ────────────────────────────────

describe('CompactionStrategyConfig — boundary values', () => {
  it('accepts maxTranscriptLines=0 (disable transcript capping)', () => {
    const raw = {
      method: 'vcc',
      vccConfig: { maxTranscriptLines: 0 },
    }
    const result = resolveCompactionConfig(raw)
    expect(result).toEqual({
      method: 'vcc',
      vccConfig: { maxTranscriptLines: 0 },
    })
  })

  it('accepts maxGoalLines=0 (disable goal capping)', () => {
    const raw = {
      method: 'vcc',
      vccConfig: { maxGoalLines: 0 },
    }
    const result = resolveCompactionConfig(raw)
    expect(result).toEqual({
      method: 'vcc',
      vccConfig: { maxGoalLines: 0 },
    })
  })

  it('accepts all vccConfig fields at 0', () => {
    const raw = {
      method: 'vcc',
      vccConfig: {
        maxTranscriptLines: 0,
        maxGoalLines: 0,
        maxFileEntries: 0,
        maxCommitEntries: 0,
        maxPreferenceLines: 0,
        maxOutstandingLines: 0,
      },
    }
    const result = resolveCompactionConfig(raw)
    expect(result).toEqual(raw)
  })
})

// ─── Zone 1: Happy path ─────────────────────────────────────

describe('CompactionStrategyConfig — happy path', () => {
  it('resolves default method with no config', () => {
    const raw = { method: 'default' }
    const result = resolveCompactionConfig(raw)
    expect(result).toEqual({ method: 'default' })
  })

  it('resolves default method with customPrompt', () => {
    const raw: CompactionStrategyConfig = {
      method: 'default',
      customPrompt: 'Focus on API interactions only.',
    }
    const result = resolveCompactionConfig(raw)
    expect(result).toEqual(raw)
  })

  it('resolves vcc method with no vccConfig', () => {
    const raw = { method: 'vcc' }
    const result = resolveCompactionConfig(raw)
    expect(result).toEqual({ method: 'vcc' })
  })

  it('resolves vcc method with partial vccConfig', () => {
    const raw: CompactionStrategyConfig = {
      method: 'vcc',
      vccConfig: { maxTranscriptLines: 80, maxGoalLines: 4 },
    }
    const result = resolveCompactionConfig(raw)
    expect(result).toEqual(raw)
  })

  it('resolves vcc method with full vccConfig', () => {
    const raw: CompactionStrategyConfig = {
      method: 'vcc',
      vccConfig: {
        maxTranscriptLines: 80,
        maxGoalLines: 4,
        maxFileEntries: 5,
        maxCommitEntries: 3,
        maxPreferenceLines: 8,
        maxOutstandingLines: 5,
      },
    }
    const result = resolveCompactionConfig(raw)
    expect(result).toEqual(raw)
  })

  it('type guard: CompactionStrategyConfig has method field', () => {
    const config: CompactionStrategyConfig = { method: 'default' }
    expect(config.method).toBe('default')
  })
})
