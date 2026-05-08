/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import {
  isAllowedOrigin,
  resetAllowedOriginsForTesting,
} from '../../../src/api/utils/cors'

describe('isAllowedOrigin', () => {
  const previousEnv = process.env.BROWSEROS_TRUSTED_ORIGINS

  beforeEach(() => {
    resetAllowedOriginsForTesting()
  })
  afterEach(() => {
    process.env.BROWSEROS_TRUSTED_ORIGINS = previousEnv
    resetAllowedOriginsForTesting()
  })

  it('accepts the pinned published extension origin even when env is empty', () => {
    process.env.BROWSEROS_TRUSTED_ORIGINS = ''
    expect(
      isAllowedOrigin('chrome-extension://bflpfmnmnokmjhmgnolecpppdbdophmk'),
    ).toBe(true)
  })

  it('rejects unknown origins when env is empty', () => {
    process.env.BROWSEROS_TRUSTED_ORIGINS = ''
    expect(isAllowedOrigin('https://example.com')).toBe(false)
    expect(isAllowedOrigin('chrome-extension://someotherid')).toBe(false)
    expect(isAllowedOrigin('null')).toBe(false)
  })

  it('accepts a single origin from env', () => {
    process.env.BROWSEROS_TRUSTED_ORIGINS = 'chrome-extension://abcdef'
    expect(isAllowedOrigin('chrome-extension://abcdef')).toBe(true)
    expect(isAllowedOrigin('chrome-extension://other')).toBe(false)
  })

  it('accepts multiple comma-separated origins and trims whitespace', () => {
    process.env.BROWSEROS_TRUSTED_ORIGINS =
      ' chrome-extension://abc , http://localhost:5173 '
    expect(isAllowedOrigin('chrome-extension://abc')).toBe(true)
    expect(isAllowedOrigin('http://localhost:5173')).toBe(true)
    expect(isAllowedOrigin('http://localhost:5174')).toBe(false)
  })

  it('is case-sensitive on origin match', () => {
    process.env.BROWSEROS_TRUSTED_ORIGINS = 'chrome-extension://abc'
    expect(isAllowedOrigin('CHROME-EXTENSION://abc')).toBe(false)
  })

  it('treats port as part of the origin', () => {
    process.env.BROWSEROS_TRUSTED_ORIGINS = 'http://localhost:5173'
    expect(isAllowedOrigin('http://localhost:5173')).toBe(true)
    expect(isAllowedOrigin('http://localhost:5174')).toBe(false)
    expect(isAllowedOrigin('http://localhost')).toBe(false)
  })

  it('rejects the literal string "null" unless explicitly allowlisted', () => {
    process.env.BROWSEROS_TRUSTED_ORIGINS = 'chrome-extension://abc'
    expect(isAllowedOrigin('null')).toBe(false)
  })

  it('drops empty entries between commas', () => {
    process.env.BROWSEROS_TRUSTED_ORIGINS = 'chrome-extension://abc,,, ,'
    expect(isAllowedOrigin('chrome-extension://abc')).toBe(true)
    expect(isAllowedOrigin('')).toBe(false)
  })
})
