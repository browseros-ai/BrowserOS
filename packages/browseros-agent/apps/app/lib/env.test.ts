import { describe, expect, it } from 'bun:test'
import { parseAlphaFeaturesFlag } from './env'

describe('parseAlphaFeaturesFlag', () => {
  it('defaults alpha features off when unset', () => {
    expect(parseAlphaFeaturesFlag(undefined)).toBe(false)
  })

  it('keeps explicit true enabled', () => {
    expect(parseAlphaFeaturesFlag('true')).toBe(true)
  })

  it('keeps explicit false disabled', () => {
    expect(parseAlphaFeaturesFlag('false')).toBe(false)
  })
})
