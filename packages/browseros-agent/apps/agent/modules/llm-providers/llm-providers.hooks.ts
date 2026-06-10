import { useEffect, useMemo, useState } from 'react'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import {
  resolveDefaultProviderId,
  resolveSelectedProvider,
} from '../../lib/llm-providers/provider-selection'
import {
  createDefaultProvidersConfig,
  DEFAULT_PROVIDER_ID,
  defaultProviderIdStorage,
  loadProviders,
  providersStorage,
} from '../../lib/llm-providers/storage'

export interface UseLlmProvidersReturn {
  providers: LlmProviderConfig[]
  defaultProviderId: string
  selectedProvider: LlmProviderConfig | null
  isLoading: boolean
  saveProvider: (provider: LlmProviderConfig) => Promise<void>
  setDefaultProvider: (providerId: string) => Promise<void>
  deleteProvider: (providerId: string) => Promise<void>
}

/** Persists the configured default provider id used by provider selection. */
// Exported only for llm-providers.hooks.test.ts; fallow's graph skips test imports.
// fallow-ignore-next-line unused-export
export async function persistDefaultProviderId(
  providerId: string,
): Promise<void> {
  await defaultProviderIdStorage.setValue(providerId)
}

/** Hook for managing LLM provider configurations. */
export function useLlmProviders(): UseLlmProvidersReturn {
  const [providers, setProviders] = useState<LlmProviderConfig[]>([])
  const [defaultProviderId, setDefaultProviderId] =
    useState<string>(DEFAULT_PROVIDER_ID)
  const [isLoading, setIsLoading] = useState(true)

  useEffect(() => {
    const loadData = async () => {
      setIsLoading(true)
      try {
        let [loadedProviders, loadedDefaultId] = await Promise.all([
          loadProviders(),
          defaultProviderIdStorage.getValue(),
        ])

        if (!loadedProviders || loadedProviders.length === 0) {
          loadedProviders = createDefaultProvidersConfig()
          await providersStorage.setValue(loadedProviders)
        }

        const resolvedDefaultId = resolveDefaultProviderId(
          loadedProviders,
          loadedDefaultId,
        )
        if (resolvedDefaultId !== loadedDefaultId) {
          await defaultProviderIdStorage.setValue(resolvedDefaultId)
        }

        setProviders(loadedProviders)
        setDefaultProviderId(resolvedDefaultId)
      } catch {
      } finally {
        setIsLoading(false)
      }
    }

    loadData()
  }, [])

  useEffect(() => {
    const unsubscribeProviders = providersStorage.watch((newProviders) => {
      if (newProviders) {
        setProviders(newProviders)
      }
    })

    const unsubscribeDefaultId = defaultProviderIdStorage.watch(
      (newDefaultId) => {
        if (newDefaultId) {
          setDefaultProviderId(newDefaultId)
        }
      },
    )

    return () => {
      unsubscribeProviders()
      unsubscribeDefaultId()
    }
  }, [])

  const saveProvider = async (provider: LlmProviderConfig) => {
    const currentProviders = (await providersStorage.getValue()) || []
    const existingIndex = currentProviders.findIndex(
      (p) => p.id === provider.id,
    )

    let updatedProviders: LlmProviderConfig[]
    if (existingIndex >= 0) {
      updatedProviders = [...currentProviders]
      updatedProviders[existingIndex] = {
        ...provider,
        updatedAt: Date.now(),
      }
    } else {
      updatedProviders = [
        ...currentProviders,
        {
          ...provider,
          createdAt: Date.now(),
          updatedAt: Date.now(),
        },
      ]
    }

    await providersStorage.setValue(updatedProviders)
  }

  const setDefaultProviderFn = async (providerId: string) => {
    setDefaultProviderId(providerId)
    await persistDefaultProviderId(providerId)
  }

  const deleteProvider = async (providerId: string) => {
    if (providerId === DEFAULT_PROVIDER_ID) {
      return
    }

    const currentProviders = (await providersStorage.getValue()) || []
    const updatedProviders = currentProviders.filter((p) => p.id !== providerId)

    if (defaultProviderId === providerId) {
      const newDefaultId = updatedProviders[0]?.id || DEFAULT_PROVIDER_ID
      await defaultProviderIdStorage.setValue(newDefaultId)
    }

    await providersStorage.setValue(updatedProviders)
  }

  const selectedProvider = useMemo(
    () => resolveSelectedProvider(providers, defaultProviderId),
    [providers, defaultProviderId],
  )

  return {
    providers,
    defaultProviderId,
    selectedProvider,
    isLoading,
    saveProvider,
    setDefaultProvider: setDefaultProviderFn,
    deleteProvider,
  }
}
