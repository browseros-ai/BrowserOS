/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * LLM config resolution - handles BROWSEROS provider lookup.
 */

import { LLM_PROVIDERS, type LLMConfig } from '@browseros/shared/schemas/llm'
import { INLINED_ENV } from '../../../env'
import { logger } from '../../logger'
import { fetchBrowserOSConfig, getLLMConfigFromProvider } from '../gateway'
import { getOAuthTokenManager } from '../oauth'
import type { ResolvedLLMConfig } from './types'

export async function resolveLLMConfig(
  config: LLMConfig,
  browserosId?: string,
): Promise<ResolvedLLMConfig> {
  // ChatGPT Pro: resolve OAuth token from server-side storage
  if (config.provider === LLM_PROVIDERS.CHATGPT_PRO) {
    return resolveChatGPTProConfig(config, browserosId)
  }

  // BrowserOS gateway: fetch config from remote service
  if (config.provider === LLM_PROVIDERS.BROWSEROS) {
    return resolveBrowserOSConfig(config, browserosId)
  }

  // All other providers: passthrough with model validation
  if (!config.model) {
    throw new Error(`model is required for ${config.provider} provider`)
  }
  return config as ResolvedLLMConfig
}

async function resolveChatGPTProConfig(
  config: LLMConfig,
  browserosId?: string,
): Promise<ResolvedLLMConfig> {
  const tokenManager = getOAuthTokenManager()
  if (!tokenManager || !browserosId) {
    throw new Error(
      'Not authenticated with ChatGPT Plus/Pro. Please login first.',
    )
  }

  const tokens = await tokenManager.refreshIfExpired('chatgpt-pro')
  if (!tokens) {
    throw new Error(
      'Not authenticated with ChatGPT Plus/Pro. Please login first.',
    )
  }

  return {
    ...config,
    model: config.model || 'gpt-5.3-codex',
    apiKey: tokens.accessToken,
    upstreamProvider: 'openai',
    accountId: tokens.accountId,
  }
}

async function resolveBrowserOSConfig(
  config: LLMConfig,
  browserosId?: string,
): Promise<ResolvedLLMConfig> {
  const configUrl = INLINED_ENV.BROWSEROS_CONFIG_URL
  if (!configUrl) {
    throw new Error(
      'BROWSEROS_CONFIG_URL environment variable is required for BrowserOS provider',
    )
  }

  logger.debug('Resolving BROWSEROS config', { configUrl, browserosId })

  const browserosConfig = await fetchBrowserOSConfig(configUrl, browserosId)
  const llmConfig = getLLMConfigFromProvider(browserosConfig, 'default')

  return {
    ...config,
    model: llmConfig.modelName,
    apiKey: llmConfig.apiKey,
    baseUrl: llmConfig.baseUrl,
    upstreamProvider: llmConfig.providerType,
  }
}
