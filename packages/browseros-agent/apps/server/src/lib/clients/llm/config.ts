/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * LLM config resolution - handles BROWSEROS provider lookup.
 */

import { LLM_PROVIDERS, type LLMConfig } from '@browseros/shared/schemas/llm'
import { getOAuthTokenManager } from '../oauth'
import { resolveMockLLMConfig, shouldUseMockLLM } from './mock-language-model'
import type { ResolvedLLMConfig } from './types'

const CHATGPT_PROVIDER_DISPLAY_NAME = 'ChatGPT'

/**
 * Provider types whose credentials come from the server, not the request.
 *
 * These take a token from this machine's oauth store, so a request naming one
 * spends something the caller never had to hold. Callers that gate on trust
 * need to know that, and this set has to mirror the branches below exactly,
 * which is why it lives beside them.
 */
export const SERVER_CREDENTIALED_PROVIDERS: ReadonlySet<string> = new Set([
  LLM_PROVIDERS.CHATGPT_PRO,
  LLM_PROVIDERS.GITHUB_COPILOT,
  LLM_PROVIDERS.QWEN_CODE,
])

export async function resolveLLMConfig(
  config: LLMConfig,
  browserosId?: string,
): Promise<ResolvedLLMConfig> {
  // Ahead of every branch below: with the harness on, nothing has to resolve a
  // credential it does not have.
  if (shouldUseMockLLM()) return resolveMockLLMConfig(config)

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
