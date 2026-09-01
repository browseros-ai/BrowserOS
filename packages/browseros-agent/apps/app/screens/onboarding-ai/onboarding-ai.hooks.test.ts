import { describe, expect, it } from 'bun:test'
import { DEFAULT_PROVIDER_ID } from '@/lib/llm-providers/provider-selection'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { AcpAgent } from '@/modules/agents/acp-agent-types'
import { countUserConnectedTargets } from './onboarding-ai.helpers'

const builtIn = { id: DEFAULT_PROVIDER_ID } as LlmProviderConfig
const mine = { id: 'p1' } as LlmProviderConfig
const other = { id: 'p2' } as LlmProviderConfig
const agent = { id: 'a1' } as AcpAgent

/**
 * The hook is an effect over two changing lists, so its rule is modelled here
 * as the same reducer the effect runs: baseline on the first ready frame, hand
 * off when the count grows past it, and only once.
 */
function runHandoff(
  frames: Array<{
    providers: LlmProviderConfig[]
    agents: AcpAgent[]
    ready: boolean
  }>,
): number {
  let baseline: number | null = null
  let handedOff = false
  let calls = 0

  for (const frame of frames) {
    if (!frame.ready || handedOff) continue
    const count = countUserConnectedTargets(frame)
    if (baseline === null) {
      baseline = count
      continue
    }
    if (count > baseline) {
      handedOff = true
      calls++
    }
  }
  return calls
}

const loading = { providers: [], agents: [], ready: false }
const fresh = { providers: [builtIn], agents: [], ready: true }
const oneProvider = { providers: [builtIn, mine], agents: [], ready: true }
const twoProviders = {
  providers: [builtIn, mine, other],
  agents: [],
  ready: true,
}
const providerAndAgent = {
  providers: [builtIn, mine],
  agents: [agent],
  ready: true,
}

describe('connection handoff rule', () => {
  it('hands off when a first provider is added', () => {
    expect(runHandoff([loading, fresh, oneProvider])).toBe(1)
  })

  it('hands off when a first agent is added', () => {
    expect(runHandoff([loading, fresh, { ...fresh, agents: [agent] }])).toBe(1)
  })

  // The bug this rule replaced: a boolean "is connected" never changes for
  // someone who already had a provider, so adding another did nothing.
  it('hands off when an already-configured user adds another provider', () => {
    expect(runHandoff([loading, oneProvider, twoProviders])).toBe(1)
  })

  it('hands off when an already-configured user adds an agent', () => {
    expect(runHandoff([loading, oneProvider, providerAndAgent])).toBe(1)
  })

  it('does nothing while the user only looks at the page', () => {
    expect(runHandoff([loading, oneProvider, oneProvider])).toBe(0)
  })

  // Removing something is a change but not a connection.
  it('does not hand off when the count shrinks', () => {
    expect(runHandoff([loading, twoProviders, oneProvider])).toBe(0)
  })

  // Both lists load on separate async chains; a baseline taken before they
  // settle would miss existing targets and fire the moment they arrived.
  it('ignores frames before the lists have settled', () => {
    expect(runHandoff([loading, loading, oneProvider, oneProvider])).toBe(0)
  })

  it('hands off only once even if the lists keep changing', () => {
    expect(runHandoff([loading, fresh, oneProvider, twoProviders, fresh])).toBe(
      1,
    )
  })

  // A subscription template leaves the page and returns, so success arrives
  // as a later change to the list rather than from a submit handler.
  it('hands off when the connection lands after a round trip', () => {
    expect(runHandoff([loading, fresh, fresh, fresh, oneProvider])).toBe(1)
  })
})
