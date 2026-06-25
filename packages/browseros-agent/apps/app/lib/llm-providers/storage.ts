import { storage } from '@wxt-dev/storage'
import { sessionStorage } from '@/lib/auth/sessionStorage'
import { getBrowserOSAdapter } from '@/lib/browseros/adapter'
import { BROWSEROS_PREFS } from '@/lib/browseros/prefs'
import {
  migrateLlmProvidersToV3,
  normalizeProviderNames,
} from './provider-name-normalization'
import {
  DEFAULT_PROVIDER_ID,
  DEFAULT_PROVIDER_NAME,
} from './provider-selection'
import type { LlmProviderConfig, LlmProvidersBackup } from './types'
import { uploadLlmProvidersToGraphql } from './uploadLlmProvidersToGraphql'

export { DEFAULT_PROVIDER_ID } from './provider-selection'

export const providersStorage = storage.defineItem<LlmProviderConfig[]>(
  'local:llm-providers',
  {
    version: 3,
    migrations: {
      2: (
        providers: LlmProviderConfig[] | null,
      ): LlmProviderConfig[] | null => {
        if (!providers) return providers
        return providers.map((provider) => {
          if (
            provider.id === DEFAULT_PROVIDER_ID &&
            provider.type === 'browseros'
          ) {
            return { ...provider, contextWindow: 200000 }
          }
          return provider
        })
      },
      3: (
        providers: LlmProviderConfig[] | null,
      ): LlmProviderConfig[] | null => {
        return migrateLlmProvidersToV3(providers)
      },
    },
  },
)

/** Mirrors provider data into BrowserOS prefs without blocking local writes. */
async function backupToBrowserOS(backup: LlmProvidersBackup): Promise<void> {
  try {
    const adapter = getBrowserOSAdapter()
    await adapter.setPref(BROWSEROS_PREFS.PROVIDERS, JSON.stringify(backup))
  } catch {
    // BrowserOS API not available - ignore
  }
}

/** Sets up one-way sync of LLM providers to BrowserOS prefs. */
export function setupLlmProvidersBackupToBrowserOS(): () => void {
  const unsubscribe = providersStorage.watch(async (providers) => {
    if (providers) {
      const defaultProviderId = await defaultProviderIdStorage.getValue()
      await backupToBrowserOS({ defaultProviderId, providers })
    }
  })
  return unsubscribe
}

/** Uploads provider metadata for signed-in users. */
export async function syncLlmProviders(): Promise<void> {
  const providers = await providersStorage.getValue()
  if (!providers || providers.length === 0) return

  const session = await sessionStorage.getValue()
  const userId = session?.user?.id
  if (!userId) return

  await uploadLlmProvidersToGraphql(providers, userId)
}

/** Sets up one-way sync of LLM providers to the GraphQL backend. */
export function setupLlmProvidersSyncToBackend(): () => void {
  syncLlmProviders().catch(() => {})

  const unsubscribe = providersStorage.watch(async () => {
    try {
      await syncLlmProviders()
    } catch {
      // Sync failed silently - will retry on next storage change
    }
  })
  return unsubscribe
}

/** Returns provider configs after applying display-name compatibility fixes. */
export async function loadProviders(): Promise<LlmProviderConfig[]> {
  const providers = (await providersStorage.getValue()) || []
  const normalizedProviders = normalizeProviderNames(providers)

  // Keep storage consistent so every consumer sees the same provider name.
  if (
    normalizedProviders.some((provider, index) => provider !== providers[index])
  ) {
    await providersStorage.setValue(normalizedProviders)
  }

  return normalizedProviders
}

/** Creates the default BrowserOS provider configuration */
export function createDefaultBrowserOSProvider(): LlmProviderConfig {
  const timestamp = Date.now()
  return {
    id: DEFAULT_PROVIDER_ID,
    type: 'browseros',
    name: DEFAULT_PROVIDER_NAME,
    baseUrl: 'https://api.browseros.com/v1',
    modelId: 'browseros-auto',
    supportsImages: true,
    contextWindow: 200000,
    temperature: 0.2,
    createdAt: timestamp,
    updatedAt: timestamp,
  }
}

/** Creates the default providers configuration. Only call when storage is empty. */
export function createDefaultProvidersConfig(): LlmProviderConfig[] {
  return [createDefaultBrowserOSProvider()]
}

export const defaultProviderIdStorage = storage.defineItem<string>(
  'local:default-provider-id',
  {
    fallback: DEFAULT_PROVIDER_ID,
  },
)
