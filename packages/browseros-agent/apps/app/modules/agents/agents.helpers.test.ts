import { describe, expect, it } from 'bun:test'
import { computeAgentsSettled } from './agents.helpers'

describe('computeAgentsSettled', () => {
  const base = {
    capabilitiesLoading: false,
    agentsSupported: true,
    urlLoading: false,
    agentsQuerySucceeded: true,
  }

  it('is false while capabilities are loading', () => {
    expect(computeAgentsSettled({ ...base, capabilitiesLoading: true })).toBe(
      false,
    )
  })

  it('is true when agents are not supported (nothing to load)', () => {
    expect(computeAgentsSettled({ ...base, agentsSupported: false })).toBe(true)
  })

  it('is false while the agent server url is still loading', () => {
    expect(computeAgentsSettled({ ...base, urlLoading: true })).toBe(false)
  })

  it('is false when the agents fetch has not succeeded (loading or failed)', () => {
    // Regression guard: a failed fetch must not be treated as authoritative,
    // otherwise the empty list would repair a persisted ACP default away.
    expect(computeAgentsSettled({ ...base, agentsQuerySucceeded: false })).toBe(
      false,
    )
  })

  it('is true only after a successful agents fetch', () => {
    expect(computeAgentsSettled(base)).toBe(true)
  })
})
