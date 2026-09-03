import type { ProviderRoutes } from '@browseros/server'
import { hc } from 'hono/client'
import { createDefaultBrowserOSProvider } from '@/lib/llm-providers/storage'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import { resolveAgentServerUrlWithRetry } from '@/modules/browseros/agent-server-url.helpers'
import { toProviderConfigs, toProviderPayload } from './llm-providers.helpers'
import { bumpProviderRevision } from './llm-providers.revision'

async function providersClient() {
  const baseUrl = await resolveAgentServerUrlWithRetry()
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
 * Loads the provider list, seeding the built-in BrowserOS provider when the
 * server has none.
 *
 * The seed lives here rather than in an effect so it can only run on a
 * confirmed empty response. Reacting to an empty list in the component would
 * fire on a failed load too, writing the default over a list that had simply
 * not arrived yet. The write is a PUT on a fixed id, so a retried fetch cannot
 * produce duplicates either.
 */
export async function fetchProviders(): Promise<LlmProviderConfig[]> {
  const configs = await listProviders()
  if (configs.length > 0) return configs

  const seeded = createDefaultBrowserOSProvider()
  await putProvider(seeded)
  return [seeded]
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
