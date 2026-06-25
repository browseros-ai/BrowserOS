/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * react-query-kit factory for the v2 audit screen. Infinite-query
 * because the dispatch log grows fast in a dogfood session and the
 * screen surfaces it via a virtualised list with on-scroll page fetch.
 *
 * Cursor pagination uses `id`-based cursors (autoincrement, monotonic
 * with insertion order) so identical-millisecond dispatches do not
 * tie and drop pages.
 */

import { createInfiniteQuery } from 'react-query-kit'
import { api } from './client'
import { parseResponse } from './parseResponse'

export interface ToolDispatchRow {
  id: number
  createdAt: number
  agentId: string
  slug: string
  agentLabel: string
  sessionId: string
  toolName: string
  pageId: number | null
  targetId: string | null
  url: string | null
  title: string | null
  argsJson: string | null
  resultMeta: string | null
  durationMs: number | null
}

export interface ListDispatchesResponse {
  rows: ToolDispatchRow[]
  nextCursor: number | null
}

export interface UseDispatchesVars {
  agentId?: string
}

export const useDispatches = createInfiniteQuery<
  ListDispatchesResponse,
  UseDispatchesVars,
  Error,
  number | undefined
>({
  queryKey: ['audit', 'dispatches'],
  fetcher: async (vars, { pageParam }) => {
    const response = await api.audit.dispatches.$get({
      query: {
        ...(vars?.agentId ? { agentId: vars.agentId } : {}),
        ...(pageParam !== undefined ? { cursor: String(pageParam) } : {}),
        limit: '100',
      },
    })
    return parseResponse<ListDispatchesResponse>(response)
  },
  initialPageParam: undefined,
  getNextPageParam: (last) => last.nextCursor ?? undefined,
  refetchInterval: 3000,
})
