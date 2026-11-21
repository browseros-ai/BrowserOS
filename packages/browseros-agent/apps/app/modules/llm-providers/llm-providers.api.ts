import { ServerApiClient, ServerApiError } from '@browseros/server-api/client'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import { resolveAgentServerUrl } from '@/modules/browseros/agent-server-url.helpers'
import { toProviderConfigs, toProviderPayload } from './llm-providers.helpers'
import { bumpProviderRevision } from './llm-providers.revision'

async function providersClient() {
  const baseUrl = await resolveAgentServerUrl()
  return new ServerApiClient(baseUrl)
}

export async function putProvider(config: LlmProviderConfig): Promise<void> {
  const client = await providersClient()
  await client.saveProvider(
    { providerId: config.id },
    toProviderPayload(config),
  )
  await bumpProviderRevision()
}

export async function deleteProvider(providerId: string): Promise<void> {
  const client = await providersClient()
  try {
    await client.deleteProvider({ providerId })
  } catch (error) {
    if (!(error instanceof ServerApiError && error.status === 404)) throw error
  }
  await bumpProviderRevision()
}

/**
 * The selected provider's id, or null when none is set.
 *
 * Held on the server beside the providers it points at, so it covers acp
 * agents as readily as llm ones. It used to sit in extension storage, which
 * meant selecting an agent left this pointing at the previous llm provider.
 */
export async function fetchDefaultProviderId(): Promise<string | null> {
  const client = await providersClient()
  const { provider } = await client.getDefaultProvider()
  return provider?.id ?? null
}

export async function putDefaultProvider(providerId: string): Promise<void> {
  const client = await providersClient()
  await client.setDefaultProvider({ providerId })
  await bumpProviderRevision()
}

export async function listProviders(): Promise<LlmProviderConfig[]> {
  const client = await providersClient()
  const { providers } = await client.listProviders()
  return toProviderConfigs(providers)
}

/**
 * The list for callers outside React, returning null when the server could not
 * be reached.
 *
 * Null rather than an empty array because the two mean different things to a
 * caller resolving an explicitly chosen provider: absent means the provider
 * was deleted and falling back is right, unreachable means the choice is
 * simply unknown and running anyway would use the wrong credentials.
 *
 * These callers must not seed either. A background alarm firing while the
 * server is still starting would otherwise write the default into a database
 * the migration had not filled yet.
 */
export async function listProvidersOrNull(): Promise<
  LlmProviderConfig[] | null
> {
  try {
    return await listProviders()
  } catch {
    return null
  }
}
