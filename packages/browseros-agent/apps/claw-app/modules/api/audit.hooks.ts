/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * react-query-kit factories for the v2 audit surfaces.
 *
 * useTasks         paginated task list (homepage + audit screen)
 * useTaskDetail    one task's full dispatch list + screenshot ids
 * useDispatches    legacy flat dispatch stream (kept for callers
 *                  that want raw rows)
 *
 * taskScreenshotUrl builds the absolute URL to the binary screenshot
 * route so an <img src> can render the persisted JPEG without going
 * through the rpc client (which is JSON-only).
 */

import { createInfiniteQuery, createQuery } from 'react-query-kit'
import { api, apiBaseUrl } from './client'
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

export type TaskStatus = 'live' | 'done' | 'failed'

export interface TaskSummary {
  sessionId: string
  agentId: string
  slug: string
  agentLabel: string
  title: string
  site: string | null
  startedAt: number
  endedAt: number | null
  durationMs: number
  dispatchCount: number
  toolSequence: string[]
  status: TaskStatus
  errorCount: number
  lastScreenshotDispatchId: number | null
  cursorId: number
}

export interface ListTasksResponse {
  tasks: TaskSummary[]
  nextCursor: number | null
}

export interface UseTasksVars {
  agentId?: string
  status?: TaskStatus
  site?: string
  search?: string
  since?: number
  limit?: number
}

export const useTasks = createInfiniteQuery<
  ListTasksResponse,
  UseTasksVars,
  Error,
  number | undefined
>({
  queryKey: ['audit', 'tasks'],
  fetcher: async (vars, { pageParam }) => {
    const query: Record<string, string> = {}
    if (vars?.agentId) query.agentId = vars.agentId
    if (vars?.status) query.status = vars.status
    if (vars?.site) query.site = vars.site
    if (vars?.search) query.search = vars.search
    if (typeof vars?.since === 'number') query.since = String(vars.since)
    if (typeof vars?.limit === 'number') query.limit = String(vars.limit)
    if (pageParam !== undefined) query.cursor = String(pageParam)
    const response = await api.audit.tasks.$get({ query })
    return parseResponse<ListTasksResponse>(response)
  },
  initialPageParam: undefined,
  getNextPageParam: (last) => last.nextCursor ?? undefined,
  refetchInterval: 3000,
  // Keep the previously-fetched pages visible while a new variable
  // set is fetching. Without this every filter / search change
  // briefly clears the table to the loading skeleton, which also
  // unmounts adjacent surfaces (FilterBar) and steals focus from
  // the search input mid-typing.
  placeholderData: (prev) => prev,
})

export interface TaskDetail extends TaskSummary {
  dispatches: ToolDispatchRow[]
  screenshotDispatchIds: number[]
  startEvent: {
    createdAt: number
    clientName: string
    clientVersion: string
  } | null
  endEvent: {
    createdAt: number
    kind: 'closed' | 'errored'
    reason: string | null
  } | null
}

export const useTaskDetail = createQuery<
  TaskDetail,
  { sessionId: string },
  Error
>({
  queryKey: ['audit', 'task'],
  fetcher: async ({ sessionId }) => {
    const response = await api.audit.tasks[':sessionId'].$get({
      param: { sessionId },
    })
    return parseResponse<TaskDetail>(response)
  },
  refetchInterval: (q) => (q.state.data?.status === 'live' ? 3000 : false),
})

/** Absolute URL for the persisted screenshot of one dispatch. */
export function taskScreenshotUrl(dispatchId: number): string {
  return `${apiBaseUrl()}/audit/screenshot/${dispatchId}`
}
