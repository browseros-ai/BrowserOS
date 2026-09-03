import type { LlmProviderRoutes } from '@browseros/server'
import { hc } from 'hono/client'
import { createDefaultBrowserOSProvider } from '@/lib/llm-providers/storage'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import { resolveAgentServerUrlWithRetry } from '@/modules/browseros/agent-server-url.helpers'
import {
  type ProviderRow,
  toProviderConfigs,
  toProviderPayload,
} from './llm-providers.helpers'

async function providersClient() {
  const baseUrl = await resolveAgentServerUrlWithRetry()
  return hc<LlmProviderRoutes>(`${baseUrl}/llm-providers`)
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
}

export async function deleteProvider(providerId: string): Promise<void> {
  const client = await providersClient()
  const response = await client[':providerId'].$delete({
    param: { providerId },
  })
  if (!response.ok && response.status !== 404) {
    throw new Error(`Failed to delete provider (${response.status})`)
  }
}

export async function listProviders(): Promise<LlmProviderConfig[]> {
  const client = await providersClient()
  const response = await client.index.$get()
  if (!response.ok) {
    throw new Error(`Failed to load providers (${response.status})`)
  }
  const { providers } = await response.json()
  return toProviderConfigs(providers as ProviderRow[])
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
 * The list for callers outside React, which fall back to the built-in provider
 * of their own accord. They must not seed: a background alarm firing while the
 * server is starting would otherwise write the default into an empty database
 * that the migration had not filled yet.
 */
export async function listProvidersOrEmpty(): Promise<LlmProviderConfig[]> {
  try {
    return await listProviders()
  } catch {
    return []
  }
}
