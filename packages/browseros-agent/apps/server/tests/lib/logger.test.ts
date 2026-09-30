/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import { shouldRotateLog } from '../../src/lib/logger'

const DAY_MS = 24 * 60 * 60 * 1000
const MB = 1024 * 1024

describe('shouldRotateLog', () => {
  it('rotates a log older than the max age', () => {
    expect(shouldRotateLog(1 * MB, 0, DAY_MS + 1)).toBe(true)
  })

  it('rotates a log larger than the size cap', () => {
    const now = 1_000_000
    expect(shouldRotateLog(60 * MB, now, now)).toBe(true)
  })

  it('leaves a small, recent log alone', () => {
    const now = 1_000_000
    expect(shouldRotateLog(1 * MB, now, now)).toBe(false)
  })

  it('does not rotate exactly at the age boundary', () => {
    expect(shouldRotateLog(1 * MB, 0, DAY_MS)).toBe(false)
  })
})
