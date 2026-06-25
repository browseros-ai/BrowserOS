import useSWR from 'swr'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'

interface McpServerResponse {
  servers: {
    name: string
    description: string
  }[]
  count: number
}

const getAllManagedServers = async ([hostUrl]: [hostUrl: string]) => {
  const response = await fetch(`${hostUrl}/klavis/servers`)
  const servers = (await response.json()) as McpServerResponse
  return servers
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
