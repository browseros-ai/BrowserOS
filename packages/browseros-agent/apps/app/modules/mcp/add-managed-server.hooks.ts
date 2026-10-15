import useSWRMutation from 'swr/mutation'
import { serverApi, serverData } from '@/lib/server-api/client'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'

const addManagedServer = async (
  url: string,
  { arg }: { arg: { serverName: string } },
) => {
  return serverData(
    serverApi(url.slice(0, -'/klavis/servers/add'.length)).http.POST(
      '/klavis/servers/add',
      { body: arg },
    ),
  )
}

export const useAddManagedServer = () => {
  const { baseUrl: agentServerUrl } = useAgentServerUrl()

  return useSWRMutation(
    agentServerUrl ? `${agentServerUrl}/klavis/servers/add` : null,
    addManagedServer,
  )
}
