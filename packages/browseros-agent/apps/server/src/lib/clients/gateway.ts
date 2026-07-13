/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { logger } from '../logger'

export interface Provider {
  name: string
  model: string
  apiKey: string
  baseUrl?: string
  dailyRateLimit?: number
  dailyCredits?: number
  creditCostPerRequest?: number
  resetInterval?: string
  providerType?: string // LLMProvider value from ai-gateway: "openrouter" | "azure" | "anthropic"
}

export interface CreditsInfo {
  credits: number
  dailyLimit: number
  lastResetAt?: string
}

export interface BrowserOSConfig {
  providers: Provider[]
}

export interface LLMConfig {
  modelName: string
  baseUrl?: string
  apiKey: string
  provider: Provider
  providerType?: string
}

export async function fetchBrowserOSConfig(
  configUrl: string,
  browserosId?: string,
): Promise<BrowserOSConfig> {
  logger.debug('Fetching Request Browser provider config', { configUrl, browserosId })

  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
  }

  if (browserosId) {
    headers['X-BrowserOS-ID'] = browserosId
  }

  try {
    const response = await fetch(configUrl, {
      method: 'GET',
      headers,
    })

    if (!response.ok) {
      const errorText = await response.text()
      throw new Error(
        `Failed to fetch config: ${response.status} ${response.statusText} - ${errorText}`,
      )
    }

    const config = (await response.json()) as BrowserOSConfig

    if (!Array.isArray(config.providers) || config.providers.length === 0) {
      throw new Error(
        'Invalid config response: providers array is empty or missing',
      )
    }

    for (const provider of config.providers) {
      if (!provider.name || !provider.model || !provider.apiKey) {
        throw new Error('Invalid provider: missing name, model, or apiKey')
      }
    }

    const defaultProvider = config.providers.find((p) => p.name === 'default')
  logger.info('✅ Request Browser provider config fetched', {
      providerCount: config.providers.length,
      dailyRateLimit: defaultProvider?.dailyRateLimit,
    })

    return config
  } catch (error) {
  logger.error('❌ Failed to fetch Request Browser provider config', {
      configUrl,
      error: error instanceof Error ? error.message : String(error),
    })
    throw error
  }
}

/**
 * Get LLM config from a provider in the BrowserOS config
 * @param config - BrowserOS config containing providers
 * @param providerName - Name of the provider to use (defaults to 'default')
 * @returns LLM config with modelName, baseUrl, apiKey, and provider
 */
export function getLLMConfigFromProvider(
  config: BrowserOSConfig,
  providerName = 'default',
): LLMConfig {
  const provider = config.providers.find((p) => p.name === providerName)

  if (!provider) {
    throw new Error(
      `Provider '${providerName}' not found in config. Available providers: ${config.providers.map((p) => p.name).join(', ')}`,
    )
  }

  return {
    modelName: provider.model,
    baseUrl: provider.baseUrl,
    apiKey: provider.apiKey,
    provider,
    providerType: provider.providerType,
  }
}

export async function fetchCredits(
  gatewayBaseUrl: string,
  browserosId: string,
): Promise<CreditsInfo> {
  const url = new URL(`/credits/${browserosId}`, gatewayBaseUrl).href
  const response = await fetch(url)
  if (!response.ok) {
    const errorText = await response.text()
    throw new Error(
      `Failed to fetch credits: ${response.status} ${response.statusText} - ${errorText}`,
    )
  }
  const result = (await response.json()) as CreditsInfo
  logger.debug('Credits fetched', { credits: result.credits })
  return result
}
