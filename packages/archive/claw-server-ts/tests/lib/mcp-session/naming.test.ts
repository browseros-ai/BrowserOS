import { describe, expect, it } from 'bun:test'
import {
  buildSessionGroupTitle,
  clientPrefixFromSlug,
  normalizeSmallName,
} from '../../../src/lib/mcp-session/naming'

describe('normalizeSmallName', () => {
  it('normalizes user labels into short lowercase task names', () => {
    expect(normalizeSmallName('Invoice Processing!')).toBe('invoice-processing')
    expect(normalizeSmallName('  LinkedIn   Jobs ')).toBe('linkedin-jobs')
    expect(normalizeSmallName('one two three four five')).toBe('one-two-three')
  })

  it('returns empty when no usable ASCII alphanumeric content remains', () => {
    expect(normalizeSmallName('!!!')).toBe('')
    expect(normalizeSmallName('')).toBe('')
    expect(normalizeSmallName('日本語')).toBe('')
  })

  it('caps long names at 32 characters', () => {
    expect(normalizeSmallName('x'.repeat(60))).toBe('x'.repeat(32))
  })
})

describe('clientPrefixFromSlug', () => {
  it('uses the first client slug token', () => {
    expect(clientPrefixFromSlug('claude-code')).toBe('claude')
    expect(clientPrefixFromSlug('cursor')).toBe('cursor')
    expect(clientPrefixFromSlug('')).toBe('agent')
  })
})

describe('buildSessionGroupTitle', () => {
  it('combines the client prefix and small session name', () => {
    expect(buildSessionGroupTitle('claude', 'invoice-processing')).toBe(
      'claude/invoice-processing',
    )
  })
})
