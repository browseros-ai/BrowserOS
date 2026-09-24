import { storage } from '@wxt-dev/storage'
import { getBrowserOSAdapter } from '@/lib/browseros/adapter'
import { BROWSEROS_PREFS } from '@/lib/browseros/prefs'
import {
  migrateLlmProvidersToV3,
  normalizeProviderNames,
} from './provider-name-normalization'
import { dropRemovedProviderConfigs } from './removed-provider-types'
import type { LlmProviderConfig, LlmProvidersBackup } from './types'

export const providersStorage = storage.defineItem<LlmProviderConfig[]>(
  'local:llm-providers',
  {
    version: 5,
    migrations: {
      // 2 widened the retired hosted provider's context window. Nothing it
      // could act on survives, and migration 5 drops the type outright.
      2: (providers: LlmProviderConfig[] | null): LlmProviderConfig[] | null =>
        providers,
      3: (
        providers: LlmProviderConfig[] | null,
      ): LlmProviderConfig[] | null => {
        return migrateLlmProvidersToV3(providers)
      },
      4: dropRemovedProviderConfigs,
      5: dropRemovedProviderConfigs,
    },
  },
)

async function backupToBrowserOS(backup: LlmProvidersBackup): Promise<void> {
  try {
    const adapter = getBrowserOSAdapter()
    await adapter.setPref(BROWSEROS_PREFS.PROVIDERS, JSON.stringify(backup))
  } catch {}
}

export function setupLlmProvidersBackupToBrowserOS(): () => void {
  const unsubscribe = providersStorage.watch(async (providers) => {
    if (providers) {
      const defaultProviderId = await defaultProviderIdStorage.getValue()
      await backupToBrowserOS({ defaultProviderId, providers })
    }
  })
  return unsubscribe
}

export async function loadProviders(): Promise<LlmProviderConfig[]> {
  const providers = (await providersStorage.getValue()) || []
  const supportedProviders = dropRemovedProviderConfigs(providers) ?? []
  const normalizedProviders = normalizeProviderNames(supportedProviders)

  if (
    supportedProviders.length !== providers.length ||
    normalizedProviders.some(
      (provider, index) => provider !== supportedProviders[index],
    )
  ) {
    await providersStorage.setValue(normalizedProviders)
  }

  return normalizedProviders
}

/**
 * Legacy local copy of the selected provider id, read only by the one-time
 * migration and by prompt refinement. Empty rather than naming the built-in
 * provider, which no longer exists.
 */
export const defaultProviderIdStorage = storage.defineItem<string>(
  'local:default-provider-id',
  {
    fallback: '',
  },
)
