import { getAgentServerUrl } from '@/lib/browseros/helpers'
import {
  createDefaultBrowserOSProvider,
  defaultProviderIdStorage,
  providersStorage,
} from '@/lib/llm-providers/storage'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import {
  findCloudChatProviderById,
  resolveCloudChatProvider,
} from '../llm-providers/provider-runtime'

const resolveProvider = async (
  providerId?: string,
): Promise<LlmProviderConfig> => {
  const providers = await providersStorage.getValue()
  if (providers?.length) {
    const explicitProvider = findCloudChatProviderById(providers, providerId)
    if (explicitProvider) return explicitProvider

    const defaultProviderId = await defaultProviderIdStorage.getValue()
    const provider = resolveCloudChatProvider(providers, defaultProviderId)
    if (provider) return provider
  }
  return createDefaultBrowserOSProvider()
}

interface RefinePromptResponse {
  success: boolean
  refined?: string
  message?: string
}

export async function refinePrompt(params: {
  prompt: string
  name: string
  providerId?: string
}): Promise<string> {
  const agentServerUrl = await getAgentServerUrl()
  const provider = await resolveProvider(params.providerId)

  const response = await fetch(`${agentServerUrl}/refine-prompt`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      prompt: params.prompt,
      name: params.name,
      provider: provider.type,
      model: provider.modelId ?? 'default',
      apiKey: provider.apiKey,
      baseUrl: provider.baseUrl,
      resourceName: provider.resourceName,
      accessKeyId: provider.accessKeyId,
      secretAccessKey: provider.secretAccessKey,
      region: provider.region,
      sessionToken: provider.sessionToken,
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
