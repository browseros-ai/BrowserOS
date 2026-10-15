import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import {
  getServerApi,
  nullOnNotFound,
  serverData,
} from '@/lib/server-api/client'
import { toProviderConfigs, toProviderPayload } from './llm-providers.helpers'
import { bumpProviderRevision } from './llm-providers.revision'

export async function putProvider(config: LlmProviderConfig): Promise<void> {
  const client = await getServerApi()
  await serverData(
    client.http.PUT('/providers/{providerId}', {
      params: { path: { providerId: config.id } },
      body: toProviderPayload(config),
    }),
    (error) => `Failed to save provider (${error.status})`,
  )
  await bumpProviderRevision()
}

export async function deleteProvider(providerId: string): Promise<void> {
  const client = await getServerApi()
  await nullOnNotFound(
    serverData(
      client.http.DELETE('/providers/{providerId}', {
        params: { path: { providerId } },
      }),
      (error) => `Failed to delete provider (${error.status})`,
    ),
  )
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
  const client = await getServerApi()
  const { provider } = await serverData(
    client.http.GET('/providers/default'),
    (error) => `Failed to load the default provider (${error.status})`,
  )
  return provider?.id ?? null
}

export async function putDefaultProvider(providerId: string): Promise<void> {
  const client = await getServerApi()
  await serverData(
    client.http.PUT('/providers/default', { body: { providerId } }),
    (error) => `Failed to set the default provider (${error.status})`,
  )
  await bumpProviderRevision()
}

export async function listProviders(): Promise<LlmProviderConfig[]> {
  const client = await getServerApi()
  const { providers } = await serverData(
    client.http.GET('/providers'),
    (error) => `Failed to load providers (${error.status})`,
  )
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
