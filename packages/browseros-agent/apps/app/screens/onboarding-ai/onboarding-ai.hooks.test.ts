import { describe, expect, it } from 'bun:test'
import { DEFAULT_PROVIDER_ID } from '@/lib/llm-providers/provider-selection'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { AcpAgent } from '@/modules/agents/acp-agent-types'
import { hasUserConnectedTarget } from './onboarding-ai.helpers'

const builtIn = { id: DEFAULT_PROVIDER_ID } as LlmProviderConfig
const mine = { id: 'p1' } as LlmProviderConfig

/**
 * The hook is an effect over a changing list, so its rule is modelled here as
 * the same reducer the effect runs: baseline on the first settled read, hand
 * off only on a false -> true transition, and only once.
 */
function runHandoff(
  frames: Array<{
    providers: LlmProviderConfig[]
    agents: AcpAgent[]
    ready: boolean
  }>,
): number {
  let wasConnected: boolean | null = null
  let handedOff = false
  let calls = 0

  for (const frame of frames) {
    if (!frame.ready || handedOff) continue
    const connected = hasUserConnectedTarget(frame)
    if (wasConnected === null) {
      wasConnected = connected
      continue
    }
    if (connected && !wasConnected) {
      handedOff = true
      calls++
    }
    wasConnected = connected
  }
  return calls
}

const loading = { providers: [], agents: [], ready: false }
const empty = { providers: [builtIn], agents: [], ready: true }
const connected = { providers: [builtIn, mine], agents: [], ready: true }

describe('connection handoff rule', () => {
  it('hands off when a provider is added', () => {
    expect(runHandoff([loading, empty, connected])).toBe(1)
  })

  it('does nothing while nothing is connected', () => {
    expect(runHandoff([loading, empty, empty])).toBe(0)
  })

  // A returning user opening this route already has providers. Firing on the
  // state rather than the transition would bounce them before it renders.
  it('does not hand off a user who arrives already connected', () => {
    expect(runHandoff([loading, connected, connected])).toBe(0)
  })

  // The lists start empty while loading; treating that as the baseline would
  // make the first settled read look like a transition.
  it('ignores frames before the lists have settled', () => {
    expect(runHandoff([loading, loading, connected, connected])).toBe(0)
  })

  it('hands off only once even if the list keeps changing', () => {
    expect(runHandoff([loading, empty, connected, empty, connected])).toBe(1)
  })

  // An OAuth template leaves the page and returns, so success arrives as a
  // later change to the list rather than from a submit handler.
  it('hands off when the connection lands after a round trip', () => {
    expect(runHandoff([loading, empty, empty, empty, connected])).toBe(1)
  })
})
