import { DEFAULT_PROVIDER_ID } from '@/lib/llm-providers/provider-selection'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { AcpAgent } from '@/modules/agents/acp-agent-types'

/**
 * How many targets the user has connected of their own.
 *
 * Not `providers.length`: `createDefaultBrowserOSProvider()` seeds a built-in
 * entry on first load, so a brand new profile already has one provider. Agents
 * count, since this step offers harnesses alongside providers and connecting
 * one is just as much a finished setup.
 *
 * A count rather than a boolean because the handoff must fire for someone who
 * already had a provider and adds another. A boolean makes that case
 * indistinguishable from idling on the page.
 */
export function countUserConnectedTargets(input: {
  providers: readonly LlmProviderConfig[]
  agents: readonly AcpAgent[]
}): number {
  const ownProviders = input.providers.filter(
    (provider) => provider.id !== DEFAULT_PROVIDER_ID,
  ).length
  return ownProviders + input.agents.length
}
