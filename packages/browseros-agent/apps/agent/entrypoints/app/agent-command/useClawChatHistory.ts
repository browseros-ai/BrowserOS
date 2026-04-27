import { useInfiniteQuery } from '@tanstack/react-query'
import { useAgentServerUrl } from '@/lib/browseros/useBrowserOSProviders'
import type { AgentHistoryPageResponse } from './claw-chat-types'

const HISTORY_QUERY_KEY = 'claw-agent-history'

async function fetchClawJson<T>(url: string): Promise<T> {
  const response = await fetch(url)

  if (!response.ok) {
    let message = `Request failed with status ${response.status}`
    try {
      const body = (await response.json()) as { error?: string }
      if (body.error) message = body.error
    } catch {}
    throw new Error(message)
  }

  return response.json() as Promise<T>
}

function buildClawUrl(baseUrl: string, path: string): URL {
  return new URL(`/claw${path}`, baseUrl)
}

export function useClawChatHistory({
  agentId,
  sessionKey,
  enabled = true,
  limit = 50,
}: {
  agentId: string
  // null lets the server resolve the most recent user-chat session for the
  // agent — avoids an extra /session round-trip and the race that came with it.
  sessionKey: string | null
  enabled?: boolean
  limit?: number
}) {
  const {
    baseUrl,
    isLoading: urlLoading,
    error: urlError,
  } = useAgentServerUrl()

  const query = useInfiniteQuery<AgentHistoryPageResponse, Error>({
    queryKey: [HISTORY_QUERY_KEY, baseUrl, agentId, sessionKey],
    initialPageParam: undefined as string | undefined,
    queryFn: async ({ pageParam }) => {
      const url = buildClawUrl(baseUrl as string, `/agents/${agentId}/history`)
      url.searchParams.set('limit', String(limit))

      if (sessionKey) {
        url.searchParams.set('sessionKey', sessionKey)
      }
      if (typeof pageParam === 'string' && pageParam) {
        url.searchParams.set('cursor', pageParam)
      }

      return fetchClawJson<AgentHistoryPageResponse>(url.toString())
    },
    getNextPageParam: (lastPage) =>
      lastPage.page.hasMore ? lastPage.page.cursor : undefined,
    enabled: enabled && Boolean(baseUrl) && !urlLoading && Boolean(agentId),
  })

  return {
    ...query,
    error: query.error ?? urlError,
    isLoading: query.isLoading || urlLoading,
  }
}
