import useSWR from 'swr'
import { serverApi, serverData } from '@/lib/server-api/client'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'

const getAllManagedServers = async ([hostUrl]: [hostUrl: string]) => {
  return serverData(serverApi(hostUrl).http.GET('/klavis/servers'))
}

export const useGetMCPServersList = () => {
  const { baseUrl: agentServerUrl } = useAgentServerUrl()

  return useSWR(
    agentServerUrl ? [agentServerUrl, 'klavis/servers'] : null,
    getAllManagedServers,
    {
      keepPreviousData: true,
    },
  )
}
