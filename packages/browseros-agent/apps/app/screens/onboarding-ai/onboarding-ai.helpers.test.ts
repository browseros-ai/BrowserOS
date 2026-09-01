import { describe, expect, it } from 'bun:test'
import { DEFAULT_PROVIDER_ID } from '@/lib/llm-providers/provider-selection'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { AcpAgent } from '@/modules/agents/acp-agent-types'
import { countUserConnectedTargets } from './onboarding-ai.helpers'

const builtIn = { id: DEFAULT_PROVIDER_ID } as LlmProviderConfig
const mine = { id: 'p1' } as LlmProviderConfig
const other = { id: 'p2' } as LlmProviderConfig
const agent = { id: 'a1' } as AcpAgent

describe('countUserConnectedTargets', () => {
  it('is zero on a brand new profile', () => {
    expect(countUserConnectedTargets({ providers: [], agents: [] })).toBe(0)
  })

  // The built-in provider is seeded on first load, so counting providers
  // directly would report every fresh profile as already set up.
  it('does not count the seeded built-in provider', () => {
    expect(
      countUserConnectedTargets({ providers: [builtIn], agents: [] }),
    ).toBe(0)
  })

  it('counts the user own providers', () => {
    expect(
      countUserConnectedTargets({ providers: [builtIn, mine], agents: [] }),
    ).toBe(1)
  })

  // This step offers harnesses alongside providers, so connecting one is a
  // finished setup just the same.
  it('counts agents alongside providers', () => {
    expect(
      countUserConnectedTargets({
        providers: [builtIn, mine, other],
        agents: [agent],
      }),
    ).toBe(3)
  })
})
