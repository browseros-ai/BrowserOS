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
import {
  resolveMockBrowserOSConfig,
  shouldUseMockBrowserOSLLM,
} from './mock-language-model'
import type { ResolvedLLMConfig } from './types'

const CHATGPT_PROVIDER_DISPLAY_NAME = 'ChatGPT'

export async function resolveLLMConfig(
  config: LLMConfig,
  browserosId?: string,
): Promise<ResolvedLLMConfig> {
  // OAuth providers: resolve token from server-side storage
  if (config.provider === LLM_PROVIDERS.CHATGPT_PRO) {
    return resolveOAuthConfig(config, browserosId, {
      providerId: 'chatgpt-pro',
      displayName: CHATGPT_PROVIDER_DISPLAY_NAME,
      defaultModel: 'gpt-5.5',
      useRefresh: true,
      extraFields: (tokens) => ({
        upstreamProvider: 'openai',
        accountId: tokens.accountId,
      }),
    })
  }
  if (config.provider === LLM_PROVIDERS.GITHUB_COPILOT) {
    return resolveOAuthConfig(config, browserosId, {
      providerId: 'github-copilot',
      displayName: 'GitHub Copilot',
      defaultModel: 'gpt-5-mini',
      useRefresh: false,
    })
  }
  if (config.provider === LLM_PROVIDERS.QWEN_CODE) {
    return resolveOAuthConfig(config, browserosId, {
      providerId: 'qwen-code',
      displayName: 'Qwen Code',
      defaultModel: 'coder-model',
      useRefresh: true,
    })
  }

  // BrowserOS gateway: fetch config from remote service
  if (config.provider === LLM_PROVIDERS.BROWSEROS) {
    if (shouldUseMockBrowserOSLLM(config)) {
      return resolveMockBrowserOSConfig(config, browserosId)
    }
    return resolveBrowserOSConfig(config, browserosId)
  }

  // All other providers: passthrough with model validation
  if (!config.model) {
    throw new Error(`model is required for ${config.provider} provider`)
  }
  return config as ResolvedLLMConfig
}

interface OAuthResolveOptions {
  providerId: string
  displayName: string
  defaultModel: string
  useRefresh: boolean
  extraFields?: (tokens: { accountId?: string }) => Record<string, unknown>
}

async function resolveOAuthConfig(
  config: LLMConfig,
  browserosId: string | undefined,
  opts: OAuthResolveOptions,
): Promise<ResolvedLLMConfig> {
  const tokenManager = getOAuthTokenManager()
  if (!tokenManager || !browserosId) {
    throw new Error(
      `Not authenticated with ${opts.displayName}. Please login first.`,
    )
  }

  const tokens = opts.useRefresh
    ? await tokenManager.refreshIfExpired(opts.providerId)
    : tokenManager.getTokens(opts.providerId)

  if (!tokens) {
    throw new Error(
      `Not authenticated with ${opts.displayName}. Please login first.`,
    )
  }

  return {
    ...config,
    model: config.model || opts.defaultModel,
    apiKey: tokens.accessToken,
    ...opts.extraFields?.(tokens),
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
    browserosId,
  }
}
