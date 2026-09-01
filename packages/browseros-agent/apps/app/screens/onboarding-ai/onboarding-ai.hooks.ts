import { useEffect, useRef } from 'react'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { AcpAgent } from '@/modules/agents/acp-agent-types'
import { hasUserConnectedTarget } from './onboarding-ai.helpers'

/**
 * Hands off to `onConnected` the first time the user connects something.
 *
 * Fires on the transition, never on the state. Two reasons: an OAuth template
 * takes the user off the page and back, so the moment of success is a change
 * in the provider list rather than the return of a submit handler; and a user
 * who opens this route with providers already configured must not be bounced
 * away before the page renders.
 */
export function useConnectionHandoff(input: {
  providers: readonly LlmProviderConfig[]
  agents: readonly AcpAgent[]
  /** False while the lists are still loading, so we do not judge too early. */
  ready: boolean
  onConnected: () => void
}): void {
  const { providers, agents, ready, onConnected } = input
  const wasConnected = useRef<boolean | null>(null)
  const handedOff = useRef(false)

  // Subscribing to an external store's transitions: the provider list changes
  // from an OAuth round trip and from dialogs this page does not own, so there
  // is no single handler to hang this on.
  useEffect(() => {
    if (!ready || handedOff.current) return

    const connected = hasUserConnectedTarget({ providers, agents })

    // First settled read establishes the baseline rather than counting as a
    // transition, which is what keeps a returning user on the page.
    if (wasConnected.current === null) {
      wasConnected.current = connected
      return
    }

    if (connected && !wasConnected.current) {
      handedOff.current = true
      onConnected()
    }
    wasConnected.current = connected
  }, [providers, agents, ready, onConnected])
}
