import { useQuery } from '@tanstack/react-query'
import { serverApi, serverData } from '@/lib/server-api/client'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'

const INTEGRATIONS_QUERY_KEY = 'klavis-user-integrations'

const getUserMCPIntegrations = async (hostUrl: string) => {
  return serverData(serverApi(hostUrl).http.GET('/klavis/user-integrations'))
}

export const useGetUserMCPIntegrations = () => {
  const { baseUrl: agentServerUrl } = useAgentServerUrl()

  const query = useQuery({
    queryKey: [INTEGRATIONS_QUERY_KEY, agentServerUrl],
    // biome-ignore lint/style/noNonNullAssertion: guarded by enabled
    queryFn: () => getUserMCPIntegrations(agentServerUrl!),
    enabled: !!agentServerUrl,
    refetchOnWindowFocus: true,
  })

  return {
    data: query.data,
    isLoading: query.isLoading,
    isFetching: query.isFetching,
    isSuccess: query.isSuccess,
    mutate: query.refetch,
  }
}
