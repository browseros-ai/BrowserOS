import { useQuery } from '@tanstack/react-query'
import { fetchHarnessAgentHistory } from '@/modules/agents/agents.hooks'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'
import type { AgentHistoryPageResponse } from './agent-chat-types'
import { mapHarnessHistoryPage } from './harness-history-mapper'

const HISTORY_QUERY_KEY = 'harness-agent-history'

export function useHarnessChatHistory(
  agentId: string,
  sessionId = 'main',
  enabled = true,
) {
  const {
    baseUrl,
    isLoading: urlLoading,
    error: urlError,
  } = useAgentServerUrl()

  const query = useQuery<AgentHistoryPageResponse, Error>({
    queryKey: [HISTORY_QUERY_KEY, baseUrl, agentId, sessionId],
    queryFn: async () => {
      return mapHarnessHistoryPage(
        await fetchHarnessAgentHistory(agentId, sessionId),
      )
    },
    enabled: Boolean(baseUrl) && !urlLoading && enabled && Boolean(agentId),
  })

  return {
    ...query,
    error: query.error ?? urlError,
    isLoading: query.isLoading || urlLoading,
  }
}
