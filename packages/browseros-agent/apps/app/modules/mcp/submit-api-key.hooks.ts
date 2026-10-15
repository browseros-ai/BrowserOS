import useSWRMutation from 'swr/mutation'
import { serverApi, serverData } from '@/lib/server-api/client'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'

const submitApiKey = async (
  url: string,
  { arg }: { arg: { serverName: string; apiKey: string; apiKeyUrl: string } },
) => {
  return serverData(
    serverApi(url.slice(0, -'/klavis/servers/submit-api-key'.length)).http.POST(
      '/klavis/servers/submit-api-key',
      { body: arg },
    ),
  )
}

export const useSubmitApiKey = () => {
  const { baseUrl: agentServerUrl } = useAgentServerUrl()

  return useSWRMutation(
    agentServerUrl ? `${agentServerUrl}/klavis/servers/submit-api-key` : null,
    submitApiKey,
  )
}
