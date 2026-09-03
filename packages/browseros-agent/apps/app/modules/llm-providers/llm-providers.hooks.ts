import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useEffect, useState } from 'react'
import { createQuery } from 'react-query-kit'
import {
  resolveDefaultProviderId,
  resolveSelectedProvider,
} from '@/lib/llm-providers/provider-selection'
import {
  DEFAULT_PROVIDER_ID,
  defaultProviderIdStorage,
} from '@/lib/llm-providers/storage'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import {
  deleteProvider as deleteProviderRow,
  fetchProviders,
  putProvider,
} from './llm-providers.api'
import { planProviderSave } from './llm-providers.helpers'

export interface UseLlmProvidersReturn {
  providers: LlmProviderConfig[]
  defaultProviderId: string
  selectedProvider: LlmProviderConfig | null
  isLoading: boolean
  /**
   * The server could not be reached, as opposed to reporting no providers.
   * Callers must not treat this as an empty list: the difference is between
   * offering to set up a first provider and saying the list is unavailable.
   */
  isUnavailable: boolean
  /**
   * Resolves with the row that was actually written. A single-instance save
   * keeps the existing provider's id, so the caller must not assume the id it
   * passed in is the one that persisted.
   */
  saveProvider: (provider: LlmProviderConfig) => Promise<LlmProviderConfig>
  setDefaultProvider: (providerId: string) => Promise<void>
  deleteProvider: (providerId: string) => Promise<void>
}

export const useProvidersQuery = createQuery<LlmProviderConfig[]>({
  queryKey: ['llm-providers'],
  fetcher: fetchProviders,
})

/** Persists the configured default provider id used by provider selection. */
export async function persistDefaultProviderId(
  providerId: string,
): Promise<void> {
  await defaultProviderIdStorage.setValue(providerId)
}

/**
 * The default provider id stays in extension storage rather than the database.
 *
 * It is a per-profile preference, and every profile on a machine shares one
 * database, so a column would make them share a default too. A stale id costs
 * nothing because `resolveDefaultProviderId` repairs it on read.
 */
function useDefaultProviderId(): [string, (id: string) => void] {
  const [defaultProviderId, setDefaultProviderId] =
    useState<string>(DEFAULT_PROVIDER_ID)

  useEffect(() => {
    let cancelled = false
    defaultProviderIdStorage.getValue().then((stored) => {
      if (!cancelled && stored) setDefaultProviderId(stored)
    })
    const unwatch = defaultProviderIdStorage.watch((next) => {
      if (next) setDefaultProviderId(next)
    })
    return () => {
      cancelled = true
      unwatch()
    }
  }, [])

  return [defaultProviderId, setDefaultProviderId]
}

/** Hook for managing LLM provider configurations. */
export function useLlmProviders(): UseLlmProvidersReturn {
  const queryClient = useQueryClient()
  const providersQuery = useProvidersQuery()
  const [storedDefaultId, setStoredDefaultId] = useDefaultProviderId()

  const providers = providersQuery.data ?? []
  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: useProvidersQuery.getKey() })

  const saveMutation = useMutation({
    mutationFn: async (provider: LlmProviderConfig) => {
      const { saved, removedIds } = planProviderSave(providers, provider)
      await putProvider(saved)
      for (const id of removedIds) await deleteProviderRow(id)
      // The row that persisted, which is not always the one passed in: a
      // single-instance save keeps the earlier provider's id, and that is the
      // id chat target selection has to reference.
      return saved
    },
    onSuccess: invalidate,
  })

  const deleteMutation = useMutation({
    mutationFn: async (providerId: string) => {
      // The built-in provider is what the app falls back to, so removing it
      // would leave nothing to chat with.
      if (providerId === DEFAULT_PROVIDER_ID) return

      // Delete first. Moving the default before the row is gone leaves the
      // provider configured but no longer default when the delete fails, with
      // nothing to tell the user it happened. The reverse is harmless: a
      // default id pointing at a deleted provider is repaired on read.
      await deleteProviderRow(providerId)

      if (storedDefaultId === providerId) {
        const nextDefault =
          providers.find((provider) => provider.id !== providerId)?.id ??
          DEFAULT_PROVIDER_ID
        setStoredDefaultId(nextDefault)
        await persistDefaultProviderId(nextDefault)
      }
    },
    onSuccess: invalidate,
  })

  const setDefaultProvider = async (providerId: string) => {
    setStoredDefaultId(providerId)
    await persistDefaultProviderId(providerId)
  }

  // Derived on read rather than repaired in storage: the write would be a side
  // effect of rendering, and every reader resolves the id the same way anyway.
  const defaultProviderId = resolveDefaultProviderId(providers, storedDefaultId)

  return {
    providers,
    defaultProviderId,
    selectedProvider: resolveSelectedProvider(providers, defaultProviderId),
    isLoading: providersQuery.isPending,
    isUnavailable: providersQuery.isError,
    saveProvider: (provider) => saveMutation.mutateAsync(provider),
    setDefaultProvider,
    deleteProvider: async (providerId) => {
      await deleteMutation.mutateAsync(providerId)
    },
  }
}
