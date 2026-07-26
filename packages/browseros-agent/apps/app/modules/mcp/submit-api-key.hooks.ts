import useSWRMutation from 'swr/mutation'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'

interface SubmitApiKeyResponse {
  success: boolean
  serverName: string
}

interface SubmitApiKeyError {
  error: string
}

const submitApiKey = async (
  url: string,
  { arg }: { arg: { serverName: string; apiKey: string; apiKeyUrl: string } },
): Promise<SubmitApiKeyResponse> => {
  const response = await fetch(url, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({
      serverName: arg.serverName,
      apiKey: arg.apiKey,
      apiKeyUrl: arg.apiKeyUrl,
    }),
  })

  if (!response.ok) {
    const errorData = (await response.json()) as SubmitApiKeyError
    throw new Error(errorData.error || 'Failed to submit API key')
  }

  return response.json() as Promise<SubmitApiKeyResponse>
}

export const useSubmitApiKey = () => {
  const { baseUrl: agentServerUrl } = useAgentServerUrl()

  return useSWRMutation(
    agentServerUrl ? `${agentServerUrl}/klavis/servers/submit-api-key` : null,
    submitApiKey,
  )
}
