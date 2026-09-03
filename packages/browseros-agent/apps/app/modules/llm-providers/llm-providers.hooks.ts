import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'
import { createQuery } from 'react-query-kit'
import {
  resolveDefaultProviderId,
  resolveSelectedProvider,
} from '@/lib/llm-providers/provider-selection'
import { DEFAULT_PROVIDER_ID } from '@/lib/llm-providers/storage'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import {
  deleteProvider as deleteProviderRow,
  fetchDefaultProviderId,
  fetchProviders,
  putDefaultProvider,
  putProvider,
} from './llm-providers.api'
import { planProviderSave } from './llm-providers.helpers'
import { watchProviderRevision } from './llm-providers.revision'

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

/**
 * The selected provider, held on the server beside the providers it points at.
 *
 * It lived in extension storage until the two provider tables were merged,
 * which meant it could only ever name an llm provider: selecting an acp agent
 * wrote the other pointer and left this one stale.
 */
export const useDefaultProviderIdQuery = createQuery<string | null>({
  queryKey: ['provider-default'],
  fetcher: fetchDefaultProviderId,
})

/** Persists the configured default provider id used by provider selection. */
export async function persistDefaultProviderId(
  providerId: string,
): Promise<void> {
  await putDefaultProvider(providerId)
}

/**
 * Keeps this surface current with provider writes made in another one.
 *
 * Each extension surface has its own query cache, and the rows live on the
 * server where nothing can watch them. Writers bump a revision in extension
 * storage instead, which does broadcast to every context, and each mounted
 * view refetches. This is what `providersStorage.watch` used to do before the
 * list moved off extension storage.
 */
function useProviderRevision(): void {
  const queryClient = useQueryClient()

  useEffect(
    () =>
      watchProviderRevision(() => {
        queryClient.invalidateQueries({ queryKey: useProvidersQuery.getKey() })
        queryClient.invalidateQueries({
          queryKey: useDefaultProviderIdQuery.getKey(),
        })
      }),
    [queryClient],
  )
}

/** Hook for managing LLM provider configurations. */
export function useLlmProviders(): UseLlmProvidersReturn {
  const queryClient = useQueryClient()
  const providersQuery = useProvidersQuery()
  const defaultQuery = useDefaultProviderIdQuery()
  useProviderRevision()
  const storedDefaultId = defaultQuery.data ?? DEFAULT_PROVIDER_ID

  const providers = providersQuery.data ?? []
  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: useProvidersQuery.getKey() })
  const invalidateDefault = () =>
    queryClient.invalidateQueries({
      queryKey: useDefaultProviderIdQuery.getKey(),
    })

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

      // Nothing to repoint by hand: deleting the row removes the default with
      // it, and the next provider is chosen on read.
    },
    onSuccess: () => {
      invalidate()
      invalidateDefault()
    },
  })

  const setDefaultMutation = useMutation({
    mutationFn: persistDefaultProviderId,
    onSuccess: invalidateDefault,
  })

  const setDefaultProvider = async (providerId: string) => {
    await setDefaultMutation.mutateAsync(providerId)
  }

  // Derived on read rather than repaired in storage: the write would be a side
  // effect of rendering, and every reader resolves the id the same way anyway.
  const defaultProviderId = resolveDefaultProviderId(providers, storedDefaultId)

  return {
    providers,
    defaultProviderId,
    selectedProvider: resolveSelectedProvider(providers, defaultProviderId),
    isLoading: providersQuery.isPending || defaultQuery.isPending,
    isUnavailable: providersQuery.isError || defaultQuery.isError,
    saveProvider: (provider) => saveMutation.mutateAsync(provider),
    setDefaultProvider,
    deleteProvider: async (providerId) => {
      await deleteMutation.mutateAsync(providerId)
    },
  }
}
