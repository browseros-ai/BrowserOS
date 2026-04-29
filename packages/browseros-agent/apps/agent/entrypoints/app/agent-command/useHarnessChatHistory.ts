import { useQuery } from '@tanstack/react-query'
import { fetchHarnessAgentHistory } from '@/entrypoints/app/agents/useAgents'
import { useAgentServerUrl } from '@/lib/browseros/useBrowserOSProviders'
import type { AgentHistoryPageResponse } from './claw-chat-types'
import { mapHarnessHistoryPage } from './harness-history-mapper'

const HISTORY_QUERY_KEY = 'harness-agent-history'

export function useHarnessChatHistory(agentId: string, enabled = true) {
  const {
    baseUrl,
    isLoading: urlLoading,
    error: urlError,
  } = useAgentServerUrl()

  const query = useQuery<AgentHistoryPageResponse, Error>({
    queryKey: [HISTORY_QUERY_KEY, baseUrl, agentId, 'main'],
    queryFn: async () => {
      return mapHarnessHistoryPage(await fetchHarnessAgentHistory(agentId))
    },
    enabled: Boolean(baseUrl) && !urlLoading && enabled && Boolean(agentId),
  })

  return {
    ...query,
    error: query.error ?? urlError,
    isLoading: query.isLoading || urlLoading,
  }
}
