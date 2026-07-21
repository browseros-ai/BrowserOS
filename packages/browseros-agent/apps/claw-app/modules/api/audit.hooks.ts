/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Session audit surface for the cockpit and audit screens.
 *
 * useSessions        paginated session history (homepage + audit screen)
 * useLiveSessions    complete connected-session snapshot for Running now
 * useSessionDetail   one session's summary + full dispatch list;
 *                    polls only while the session is live
 *
 * The URL helpers let <img src> render binary JPEG routes without
 * routing those payloads through the JSON client.
 */

import {
  type Dispatch,
  type SessionDetail,
  type SessionList,
  type SessionScreenshotList,
  SessionStatus,
  type SessionSummary,
} from '@browseros/claw-api'
import {
  buildSessionPreviewUrl,
  buildSessionScreenshotUrl,
} from '@browseros/claw-api-client'
import { useEffect, useState } from 'react'
import { createInfiniteQuery, createQuery } from 'react-query-kit'
import { apiBaseUrl, apiClient, resolveApiBaseUrl } from './client'

// The screens speak task-*; the contract speaks session-*. Aliased here
// so call sites keep their vocabulary while the shapes stay canonical.
export type ToolDispatchRow = Dispatch
export type TaskStatus = SessionStatus
export type TaskSummary = SessionSummary
export type TaskDetail = SessionDetail

export interface UseSessionsVariables {
  profileId?: string
  slug?: string
  status?: SessionStatus
  site?: string
  search?: string
  since?: number
  limit?: number
}

export const useSessions = createInfiniteQuery<
  SessionList,
  UseSessionsVariables,
  Error,
  number | undefined
>({
  queryKey: ['api', 'sessions'],
  fetcher: async (variables, { pageParam }) =>
    (await apiClient()).listSessions({
      ...variables,
      ...(pageParam === undefined ? {} : { cursor: pageParam }),
    }),
  initialPageParam: undefined,
  getNextPageParam: (last) => last.nextCursor,
  refetchInterval: 3000,
  // Keep the prior pages visible while a new filter set loads so the
  // adjacent filter controls remain mounted and retain keyboard focus.
  placeholderData: (previous) => previous,
})

export const useLiveSessions = createQuery<SessionList>({
  queryKey: ['api', 'sessions', 'live'],
  fetcher: async () =>
    (await apiClient()).listSessions({ status: SessionStatus.Live }),
  refetchInterval: 1500,
})

export const useSessionDetail = createQuery<
  SessionDetail,
  { sessionId: string },
  Error
>({
  queryKey: ['api', 'session'],
  fetcher: async ({ sessionId }) =>
    (await apiClient()).getSession({ sessionId }),
  refetchInterval: (query) =>
    query.state.data?.session.status === 'live' ? 3000 : false,
})

export const useSessionScreenshots = createQuery<
  SessionScreenshotList,
  { sessionId: string },
  Error
>({
  queryKey: ['api', 'session', 'screenshots'],
  fetcher: async ({ sessionId }) =>
    (await apiClient()).listSessionScreenshots({ sessionId }),
  refetchInterval: 3000,
})

/** Absolute URL for one immutable session-owned screenshot. */
export function taskScreenshotUrl(
  sessionId: string,
  screenshotId: number,
  baseUrl = apiBaseUrl(),
): string {
  return buildSessionScreenshotUrl(baseUrl, { sessionId, screenshotId })
}

/** Absolute URL for a fresh live-session JPEG; `refresh` only busts browser caches. */
export function sessionPreviewUrl(
  sessionId: string,
  refresh: number,
  baseUrl = apiBaseUrl(),
): string {
  return buildSessionPreviewUrl(baseUrl, { sessionId, refresh })
}

/**
 * API base that follows the BrowserOS server-port pref. The pref API is
 * callback-based, so synchronous URL helpers cannot see it directly.
 */
function useResolvedApiBaseUrl(): string | null {
  const [baseUrl, setBaseUrl] = useState<string | null>(null)

  useEffect(() => {
    let active = true
    resolveApiBaseUrl().then((resolved) => {
      if (active) setBaseUrl(resolved)
    })
    return () => {
      active = false
    }
  }, [])

  return baseUrl
}

export function useTaskScreenshotBaseUrl(): string | null {
  return useResolvedApiBaseUrl()
}

export function useSessionPreviewUrl(
  sessionId: string,
  refresh: number,
): string | null {
  const baseUrl = useResolvedApiBaseUrl()
  return baseUrl === null
    ? null
    : sessionPreviewUrl(sessionId, refresh, baseUrl)
}
