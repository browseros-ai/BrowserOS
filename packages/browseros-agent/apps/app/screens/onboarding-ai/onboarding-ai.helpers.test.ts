import { describe, expect, it } from 'bun:test'
import { DEFAULT_PROVIDER_ID } from '@/lib/llm-providers/provider-selection'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { AcpAgent } from '@/modules/agents/acp-agent-types'
import { hasUserConnectedTarget } from './onboarding-ai.helpers'

const builtIn = {
  id: DEFAULT_PROVIDER_ID,
  type: 'browseros',
} as LlmProviderConfig
const mine = { id: 'p1', type: 'openai' } as LlmProviderConfig
const agent = { id: 'a1', type: 'codex' } as AcpAgent

describe('hasUserConnectedTarget', () => {
  it('is false on a brand new profile', () => {
    expect(hasUserConnectedTarget({ providers: [], agents: [] })).toBe(false)
  })

  // The built-in provider is seeded on first load, so counting providers would
  // report every fresh profile as already set up.
  it('does not count the seeded built-in provider', () => {
    expect(hasUserConnectedTarget({ providers: [builtIn], agents: [] })).toBe(
      false,
    )
  })

  it('is true once the user adds their own provider', () => {
    expect(
      hasUserConnectedTarget({ providers: [builtIn, mine], agents: [] }),
    ).toBe(true)
  })

  // This step offers harnesses alongside providers, so connecting one is a
  // finished setup just the same.
  it('is true when only a coding agent is connected', () => {
    expect(
      hasUserConnectedTarget({ providers: [builtIn], agents: [agent] }),
    ).toBe(true)
  })
})
