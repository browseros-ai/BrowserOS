/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * React Query hooks backing the per-agent Outputs rail and the
 * inline artifact card.
 *
 * Live updates: the consumer of `useAgentConversation` (see Phase 5)
 * is expected to call `useInvalidateAgentOutputs(agentId)` whenever
 * an assistant turn completes, so the rail picks up the new
 * `produced_files` rows the server attributed during that turn.
 * No SSE channel here — invalidation off the existing chat-stream
 * completion is enough for v1.
 */

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  AGENT_QUERY_KEYS,
  agentsFetch,
} from '@/entrypoints/app/agents/useAgents'
import { useAgentServerUrl } from '@/lib/browseros/useBrowserOSProviders'
import type { ProducedFile, ProducedFilesRailGroup } from './types'

interface OutputsResponse {
  groups: ProducedFilesRailGroup[]
}

interface TurnFilesResponse {
  files: ProducedFile[]
}

export function useAgentOutputs(agentId: string, enabled = true) {
  const {
    baseUrl,
    isLoading: urlLoading,
    error: urlError,
  } = useAgentServerUrl()

  const query = useQuery<ProducedFilesRailGroup[], Error>({
    queryKey: [AGENT_QUERY_KEYS.agentOutputs, baseUrl, agentId],
    queryFn: async () => {
      const data = await agentsFetch<OutputsResponse>(
        baseUrl as string,
        `/${encodeURIComponent(agentId)}/files`,
      )
      return data.groups ?? []
    },
    enabled: Boolean(baseUrl) && !urlLoading && enabled && Boolean(agentId),
  })

  return {
    groups: query.data ?? [],
    loading: query.isLoading || urlLoading,
    error: query.error ?? urlError,
    refetch: query.refetch,
  }
}

/**
 * Per-turn fetch for the inline artifact card. Used both as the
 * fallback when an SSE `produced_files` event was missed, and to
 * rehydrate a turn the user scrolled back to.
 */
export function useAgentTurnFiles(
  agentId: string,
  turnId: string | null,
  enabled = true,
) {
  const {
    baseUrl,
    isLoading: urlLoading,
    error: urlError,
  } = useAgentServerUrl()

  const query = useQuery<ProducedFile[], Error>({
    queryKey: [AGENT_QUERY_KEYS.agentTurnFiles, baseUrl, agentId, turnId],
    queryFn: async () => {
      const data = await agentsFetch<TurnFilesResponse>(
        baseUrl as string,
        `/${encodeURIComponent(agentId)}/files/turn/${encodeURIComponent(
          turnId as string,
        )}`,
      )
      return data.files ?? []
    },
    enabled:
      Boolean(baseUrl) &&
      !urlLoading &&
      enabled &&
      Boolean(agentId) &&
      Boolean(turnId),
  })

  return {
    files: query.data ?? [],
    loading: query.isLoading || urlLoading,
    error: query.error ?? urlError,
    refetch: query.refetch,
  }
}

/**
 * Returns a callable that invalidates outputs / turn-files queries
 * for one agent across any baseUrl. Call after an assistant turn
 * completes so the rail (and the inline file-card strip) pick up
 * the new attributed rows. Cheap when the queries aren't mounted
 * — react-query just marks the cached value stale.
 *
 * Implementation note: react-query's `invalidateQueries({ queryKey })`
 * does positional partial-match, so passing `undefined` as the
 * baseUrl placeholder does NOT match a cached `[…, baseUrl, …]`
 * key — the cache stayed stale. Use a predicate so we ignore the
 * baseUrl position entirely.
 */
export function useInvalidateAgentOutputs() {
  const queryClient = useQueryClient()
  return async (agentId: string, turnId?: string) => {
    await Promise.all([
      queryClient.invalidateQueries({
        predicate: (query) => {
          const key = query.queryKey
          return (
            Array.isArray(key) &&
            key[0] === AGENT_QUERY_KEYS.agentOutputs &&
            key[2] === agentId
          )
        },
      }),
      queryClient.invalidateQueries({
        predicate: (query) => {
          const key = query.queryKey
          if (
            !Array.isArray(key) ||
            key[0] !== AGENT_QUERY_KEYS.agentTurnFiles ||
            key[2] !== agentId
          ) {
            return false
          }
          // When a turnId was supplied, scope to just that turn's
          // entry. Otherwise flush every cached turn for this agent.
          return turnId ? key[3] === turnId : true
        },
      }),
    ])
  }
}

/**
 * Tiny mutation wrapper so the Outputs rail's "Refresh" button can
 * surface an `isPending` indicator while the new query is in flight.
 * No body — just triggers `refetch` on the rail's query for this
 * agent and resolves when it settles.
 */
export function useRefreshAgentOutputs(agentId: string) {
  const queryClient = useQueryClient()
  const { baseUrl } = useAgentServerUrl()
  return useMutation({
    mutationFn: async () => {
      await queryClient.refetchQueries({
        queryKey: [AGENT_QUERY_KEYS.agentOutputs, baseUrl, agentId],
        exact: true,
      })
    },
  })
}
