import useSWRMutation from 'swr/mutation'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'

interface AddServerResponse {
  success: boolean
  serverName: string
  strataId: string
  addedServers: string[]
  oauthUrl?: string
  apiKeyUrl?: string
}

interface AddServerError {
  error: string
}

const addManagedServer = async (
  url: string,
  { arg }: { arg: { serverName: string } },
): Promise<AddServerResponse> => {
  const response = await fetch(url, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ serverName: arg.serverName }),
  })

  if (!response.ok) {
    const errorData = (await response.json()) as AddServerError
    throw new Error(errorData.error || 'Failed to add server')
  }

  return response.json() as Promise<AddServerResponse>
}

export const useAddManagedServer = () => {
  const { baseUrl: agentServerUrl } = useAgentServerUrl()

  return useSWRMutation(
    agentServerUrl ? `${agentServerUrl}/klavis/servers/add` : null,
    addManagedServer,
  )
}
