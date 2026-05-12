/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Compaction strategy config resolution and validation.
 */

import { logger } from '../lib/logger'
import type { CompactionStrategyConfig } from './types'

const VALID_METHODS = new Set(['default', 'vcc'])

/**
 * Validates and resolves a raw compaction config object.
 * Returns `undefined` if the input is falsy or an empty object (no compaction override).
 * Throws on invalid configs.
 */
// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: validation function with many branches
export function resolveCompactionConfig(
  raw: unknown,
): CompactionStrategyConfig | undefined {
  if (raw == null || typeof raw !== 'object') {
    return undefined
  }

  const input = raw as Record<string, unknown>
  const keys = Object.keys(input)

  // Empty object → no override
  if (keys.length === 0) {
    return undefined
  }

  // Validate method
  const method = input.method
  if (method == null || typeof method !== 'string') {
    throw new Error(
      'Compaction config: "method" must be a string ("default" | "vcc")',
    )
  }

  if (!VALID_METHODS.has(method)) {
    throw new Error(
      `Compaction config: invalid method "${method}". Must be "default" or "vcc"`,
    )
  }

  // Validate customPrompt (if present)
  if ('customPrompt' in input) {
    if (
      typeof input.customPrompt === 'string' &&
      input.customPrompt.trim() === ''
    ) {
      throw new Error('Compaction config: customPrompt must not be blank')
    }
  }

  // Validate vccConfig (if present)
  const numericFields = [
    'maxTranscriptLines',
    'maxGoalLines',
    'maxFileEntries',
    'maxCommitEntries',
    'maxPreferenceLines',
    'maxOutstandingLines',
  ] as const

  if (
    'vccConfig' in input &&
    input.vccConfig != null &&
    typeof input.vccConfig === 'object'
  ) {
    const vcc = input.vccConfig as Record<string, unknown>
    for (const field of numericFields) {
      if (field in vcc) {
        const value = vcc[field]
        if (typeof value !== 'number' || value < 0) {
          throw new Error(
            `Compaction config: vccConfig.${field} must be a non-negative number`,
          )
        }
      }
    }
  }

  // Warn on mismatched combinations
  if (method === 'default' && 'vccConfig' in input && input.vccConfig != null) {
    logger.warn(
      '[compaction] vccConfig is set but method is "default" — vccConfig will be ignored',
    )
  }
  if (
    method === 'vcc' &&
    'customPrompt' in input &&
    typeof input.customPrompt === 'string'
  ) {
    logger.warn(
      '[compaction] customPrompt is set but method is "vcc" — customPrompt will be ignored',
    )
  }

  // Build validated output
  const result: CompactionStrategyConfig = {
    method: method as 'default' | 'vcc',
  }

  if ('customPrompt' in input && typeof input.customPrompt === 'string') {
    result.customPrompt = input.customPrompt
  }

  if (
    'vccConfig' in input &&
    input.vccConfig != null &&
    typeof input.vccConfig === 'object'
  ) {
    result.vccConfig = {}
    const vcc = input.vccConfig as Record<string, unknown>
    for (const field of numericFields) {
      if (field in vcc && typeof vcc[field] === 'number') {
        ;(result.vccConfig as Record<string, number>)[field] = vcc[field]
      }
    }
  }

  return result
}
