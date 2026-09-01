import { DEFAULT_PROVIDER_ID } from '@/lib/llm-providers/provider-selection'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { AcpAgent } from '@/modules/agents/acp-agent-types'

/**
 * Whether the user has connected anything of their own.
 *
 * Cannot be `providers.length > 0`: `createDefaultBrowserOSProvider()` seeds a
 * built-in entry on first load, so a brand new profile already has one
 * provider. A coding agent counts, since this step offers harnesses alongside
 * providers and connecting one is just as much a finished setup.
 */
export function hasUserConnectedTarget(input: {
  providers: readonly LlmProviderConfig[]
  agents: readonly AcpAgent[]
}): boolean {
  return (
    input.providers.some((provider) => provider.id !== DEFAULT_PROVIDER_ID) ||
    input.agents.length > 0
  )
}
