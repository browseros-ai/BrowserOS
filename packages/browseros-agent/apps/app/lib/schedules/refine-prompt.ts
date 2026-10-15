import { getAgentServerUrl } from '@/lib/browseros/helpers'
import { serverApi, serverData } from '@/lib/server-api/client'

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

  const data = await serverData(
    serverApi(agentServerUrl).http.POST('/refine-prompt', {
      body: {
        prompt: params.prompt,
        name: params.name,
        providerId: params.providerId,
      },
    }),
    async (error, response) => {
      const body: unknown = await response
        ?.clone()
        .json()
        .catch(() => null)
      return body &&
        typeof body === 'object' &&
        'message' in body &&
        typeof body.message === 'string'
        ? body.message
        : `Request failed: ${error.status}`
    },
  )
  if (!data.success || !data.refined)
    throw new Error(
      'message' in data && typeof data.message === 'string'
        ? data.message
        : 'Failed to refine prompt',
    )

  return data.refined
}
