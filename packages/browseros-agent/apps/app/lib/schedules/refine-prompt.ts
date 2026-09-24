import { getAgentServerUrl } from '@/lib/browseros/helpers'

interface RefinePromptResponse {
  success: boolean
  refined?: string
  message?: string
}

/**
 * Asks the server to rewrite a scheduled task's prompt.
 *
 * The provider is named and nothing more, the way /chat names one. This used
 * to resolve the provider here and post its whole configuration, so the API
 * key, the AWS secret and the session token crossed the wire on every refine.
 * The server holds the list and which one is selected, so it resolves them.
 */
export async function refinePrompt(params: {
  prompt: string
  name: string
  providerId?: string
}): Promise<string> {
  const agentServerUrl = await getAgentServerUrl()

  const response = await fetch(`${agentServerUrl}/refine-prompt`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      prompt: params.prompt,
      name: params.name,
      // Absent when the caller has none, which tells the server to use the
      // selected provider.
      providerId: params.providerId,
    }),
  })

  if (!response.ok) {
    const errorData = (await response
      .json()
      .catch(() => null)) as RefinePromptResponse | null
    throw new Error(errorData?.message ?? `Request failed: ${response.status}`)
  }

  const data = (await response.json()) as RefinePromptResponse
  if (!data.success || !data.refined) {
    throw new Error(data.message ?? 'Failed to refine prompt')
  }

  return data.refined
}
