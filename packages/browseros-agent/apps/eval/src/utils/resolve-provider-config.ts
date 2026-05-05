import {
  fetchBrowserOSConfig,
  getLLMConfigFromProvider,
} from '@browseros/server/lib/clients/gateway'
import { LLM_PROVIDERS, type LLMConfig } from '@browseros/shared/schemas/llm'
import { resolveEnvValue } from './resolve-env'

export interface ResolvedProviderConfig extends LLMConfig {
  upstreamProvider?: string
}

export async function resolveProviderConfig(
  agent: LLMConfig,
): Promise<ResolvedProviderConfig> {
  if (agent.provider === LLM_PROVIDERS.BROWSEROS) {
    const configUrl = process.env.BROWSEROS_CONFIG_URL
    if (!configUrl) {
      throw new Error(
        'BROWSEROS_CONFIG_URL environment variable is required for BrowserOS provider',
      )
    }
    const browserosConfig = await fetchBrowserOSConfig(configUrl)
    const llmConfig = getLLMConfigFromProvider(browserosConfig, 'default')
    return {
      provider: LLM_PROVIDERS.BROWSEROS,
      model: llmConfig.modelName,
      apiKey: llmConfig.apiKey,
      baseUrl: llmConfig.baseUrl,
      upstreamProvider: llmConfig.providerType,
    }
  }

  return {
    ...agent,
    apiKey: resolveEnvValue(agent.apiKey),
    accessKeyId: resolveEnvValue(agent.accessKeyId),
    secretAccessKey: resolveEnvValue(agent.secretAccessKey),
    sessionToken: resolveEnvValue(agent.sessionToken),
    region: resolveEnvValue(agent.region),
  }
}
