import { useEffect, useRef } from 'react'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { AcpAgent } from '@/modules/agents/acp-agent-types'
import { countUserConnectedTargets } from './onboarding-ai.helpers'

/**
 * Hands off to `onConnected` once the user connects something on this visit.
 *
 * Compares against a baseline taken when the lists first settle, so it fires
 * for a user who already had a provider and adds another. An earlier version
 * fired only on a not-connected to connected transition, which meant anyone
 * arriving with a provider already configured could never hand off.
 *
 * Deleting does not trigger it: the count has to grow, not merely change.
 *
 * The baseline needs both lists settled, not just the providers. They load on
 * separate async chains, and `useAcpAgents` documents that its `loading` flag
 * briefly reads false while the list is still empty, so a baseline taken too
 * early would miss existing agents and hand off the moment they arrived.
 */
export function useConnectionHandoff(input: {
  providers: readonly LlmProviderConfig[]
  agents: readonly AcpAgent[]
  /** Both lists have loaded. A baseline taken before this is meaningless. */
  ready: boolean
  onConnected: () => void
}): void {
  const { providers, agents, ready, onConnected } = input
  const baseline = useRef<number | null>(null)
  const handedOff = useRef(false)

  // Watching an external store: the lists change from an OAuth round trip and
  // from dialogs this page does not own, so there is no single handler to
  // hang this on.
  useEffect(() => {
    if (!ready || handedOff.current) return

    const count = countUserConnectedTargets({ providers, agents })

    if (baseline.current === null) {
      baseline.current = count
      return
    }

    if (count > baseline.current) {
      handedOff.current = true
      onConnected()
    }
  }, [providers, agents, ready, onConnected])
}
