import { useQuery } from '@tanstack/react-query'
import { useAgentServerUrl } from '@/lib/browseros/useBrowserOSProviders'

interface UserMCPIntegrationsList {
  integrations: {
    name: string
    is_authenticated: boolean
  }[]
  count: number
}

export const INTEGRATIONS_QUERY_KEY = 'klavis-user-integrations'

const getUserMCPIntegrations = async (
  hostUrl: string,
): Promise<UserMCPIntegrationsList> => {
  const response = await fetch(`${hostUrl}/klavis/user-integrations`)
  const data = (await response.json()) as UserMCPIntegrationsList
  return data
}

export const useGetUserMCPIntegrations = () => {
  const { baseUrl: agentServerUrl } = useAgentServerUrl()

  const query = useQuery({
    queryKey: [INTEGRATIONS_QUERY_KEY, agentServerUrl],
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
