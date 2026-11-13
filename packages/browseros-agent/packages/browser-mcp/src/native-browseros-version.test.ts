import { describe, expect, it } from 'bun:test'
import { supportsClosedTabGroups } from './native-browseros-version'

describe('supportsClosedTabGroups', () => {
  it('accepts supported two, three, and four component versions', () => {
    for (const version of [
      '0.52.2',
      '0.52.2.0',
      '0.52.2.1',
      '0.53',
      '0.53.0.0',
    ]) {
      expect(supportsClosedTabGroups(version)).toBe(true)
    }
  })

  it('compares version components numerically', () => {
    expect(supportsClosedTabGroups('0.52.10')).toBe(true)
    expect(supportsClosedTabGroups('0.52.1.10')).toBe(false)
    expect(supportsClosedTabGroups('0.51.10.0')).toBe(false)
  })

  it('fails closed for missing, blank, and malformed versions', () => {
    for (const version of [
      undefined,
      '',
      '   ',
      'unknown',
      '0.52.x.0',
      '0.52.2.0.1',
      '0.52.-1.0',
      '0.52.4294967296',
    ]) {
      expect(supportsClosedTabGroups(version)).toBe(false)
    }
  })
})
