import { useEffect, useMemo, useState } from 'react'
import {
  createDefaultProvidersConfig,
  DEFAULT_PROVIDER_ID,
  defaultProviderIdStorage,
  loadProviders,
  providersStorage,
} from './storage'
import type { LlmProviderConfig } from './types'

/**
 * Hook return type
 * @public
 */
export interface UseLlmProvidersReturn {
  /** All configured providers */
  providers: LlmProviderConfig[]
  /** ID of the default provider */
  defaultProviderId: string
  /** Full config of the currently selected provider */
  selectedProvider: LlmProviderConfig | null
  /** Whether data is loading */
  isLoading: boolean
  /** Save or update a provider */
  saveProvider: (provider: LlmProviderConfig) => Promise<void>
  /** Set the default provider */
  setDefaultProvider: (providerId: string) => Promise<void>
  /** Delete a provider */
  deleteProvider: (providerId: string) => Promise<void>
}

/**
 * Hook for managing LLM provider configurations
 * @public
 */
export function useLlmProviders(): UseLlmProvidersReturn {
  const [providers, setProviders] = useState<LlmProviderConfig[]>([])
  const [defaultProviderId, setDefaultProviderId] =
    useState<string>(DEFAULT_PROVIDER_ID)
  const [isLoading, setIsLoading] = useState(true)

  // Load initial data
  useEffect(() => {
    const loadData = async () => {
      setIsLoading(true)
      try {
        let [loadedProviders, loadedDefaultId] = await Promise.all([
          loadProviders(),
          defaultProviderIdStorage.getValue(),
        ])

        // Initialize with defaults if storage is empty
        if (!loadedProviders || loadedProviders.length === 0) {
          loadedProviders = createDefaultProvidersConfig()
          await providersStorage.setValue(loadedProviders)
        }

        if (!loadedDefaultId) {
          loadedDefaultId = DEFAULT_PROVIDER_ID
          await defaultProviderIdStorage.setValue(loadedDefaultId)
        }

        // Repair stale default ID that doesn't match any provider
        const defaultExists = loadedProviders.some(
          (p) => p.id === loadedDefaultId,
        )
        if (!defaultExists && loadedProviders.length > 0) {
          loadedDefaultId = loadedProviders[0].id
          await defaultProviderIdStorage.setValue(loadedDefaultId)
        }

        setProviders(loadedProviders)
        setDefaultProviderId(loadedDefaultId)
      } catch {
        // TODO: Record error to error recording service
      } finally {
        setIsLoading(false)
      }
    }

    loadData()
  }, [])

  // Listen for storage changes
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
      // Update existing provider
      updatedProviders = [...currentProviders]
      updatedProviders[existingIndex] = {
        ...provider,
        updatedAt: Date.now(),
      }
    } else {
      // Add new provider
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
    await defaultProviderIdStorage.setValue(providerId)
  }

  const deleteProvider = async (providerId: string) => {
    // Prevent deletion of built-in BrowserOS provider
    if (providerId === DEFAULT_PROVIDER_ID) {
      return
    }

    const currentProviders = (await providersStorage.getValue()) || []
    const updatedProviders = currentProviders.filter((p) => p.id !== providerId)

    // Handle default provider reassignment if deleted provider was default
    if (defaultProviderId === providerId) {
      const newDefaultId = updatedProviders[0]?.id || DEFAULT_PROVIDER_ID
      await defaultProviderIdStorage.setValue(newDefaultId)
    }

    await providersStorage.setValue(updatedProviders)
  }

  // Fall back to first provider if defaultProviderId is stale/invalid
  const selectedProvider = useMemo(
    () =>
      providers.find((p) => p.id === defaultProviderId) ?? providers[0] ?? null,
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
