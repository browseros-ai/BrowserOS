import type { ProviderRoutes } from '@browseros/server'
import { hc } from 'hono/client'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import { resolveAgentServerUrl } from '@/modules/browseros/agent-server-url.helpers'
import { toProviderConfigs, toProviderPayload } from './llm-providers.helpers'
import { bumpProviderRevision } from './llm-providers.revision'

async function providersClient() {
  const baseUrl = await resolveAgentServerUrl()
  return hc<ProviderRoutes>(`${baseUrl}/providers`)
}

export async function putProvider(config: LlmProviderConfig): Promise<void> {
  const client = await providersClient()
  const response = await client[':providerId'].$put({
    param: { providerId: config.id },
    json: toProviderPayload(config),
  })
  if (!response.ok) {
    throw new Error(`Failed to save provider (${response.status})`)
  }
  await bumpProviderRevision()
}

export async function deleteProvider(providerId: string): Promise<void> {
  const client = await providersClient()
  const response = await client[':providerId'].$delete({
    param: { providerId },
  })
  if (!response.ok && response.status !== 404) {
    throw new Error(`Failed to delete provider (${response.status})`)
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
  const response = await client.default.$get()
  if (!response.ok) {
    throw new Error(`Failed to load the default provider (${response.status})`)
  }
  const { provider } = await response.json()
  return provider?.id ?? null
}

export async function putDefaultProvider(providerId: string): Promise<void> {
  const client = await providersClient()
  const response = await client.default.$put({ json: { providerId } })
  if (!response.ok) {
    throw new Error(`Failed to set the default provider (${response.status})`)
  }
  await bumpProviderRevision()
}

export async function listProviders(): Promise<LlmProviderConfig[]> {
  const client = await providersClient()
  const response = await client.index.$get()
  if (!response.ok) {
    throw new Error(`Failed to load providers (${response.status})`)
  }
  const { providers } = await response.json()
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
