import useSWRMutation from 'swr/mutation'
import { serverApi, serverData } from '@/lib/server-api/client'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'

const removeManagedServer = async (
  url: string,
  { arg }: { arg: { serverName: string } },
) => {
  return serverData(
    serverApi(url.slice(0, -'/klavis/servers/remove'.length)).http.DELETE(
      '/klavis/servers/remove',
      { body: arg },
    ),
  )
}

export const useRemoveManagedServer = () => {
  const { baseUrl: agentServerUrl } = useAgentServerUrl()

  return useSWRMutation(
    agentServerUrl ? `${agentServerUrl}/klavis/servers/remove` : null,
    removeManagedServer,
  )
}
