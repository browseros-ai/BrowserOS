/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * LLM provider creation - creates Vercel AI SDK language models.
 */

import { createAmazonBedrock } from '@ai-sdk/amazon-bedrock'
import { createAnthropic } from '@ai-sdk/anthropic'
import { createAzure } from '@ai-sdk/azure'
import { createGoogleGenerativeAI } from '@ai-sdk/google'
import { createOpenAI } from '@ai-sdk/openai'
import { createOpenAICompatible } from '@ai-sdk/openai-compatible'
import { EXTERNAL_URLS } from '@browseros/shared/constants/urls'
import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'
import { createOpenRouter } from '@openrouter/ai-sdk-provider'
import type { LanguageModel } from 'ai'
import { createBrowserOSFetch } from '../../browseros-fetch'
import { logger } from '../../logger'
import { createOpenRouterCompatibleFetch } from '../../openrouter-fetch'
import { createCodexFetch } from '../oauth/codex-fetch'
import { createCopilotFetch } from '../oauth/copilot-fetch'
import type { ResolvedLLMConfig } from './types'

type ProviderFactory = (config: ResolvedLLMConfig) => LanguageModel

function createAnthropicModel(config: ResolvedLLMConfig): LanguageModel {
  if (!config.apiKey) throw new Error('Anthropic provider requires apiKey')
  return createAnthropic({ apiKey: config.apiKey })(config.model)
}

function createOpenAIModel(config: ResolvedLLMConfig): LanguageModel {
  if (!config.apiKey) throw new Error('OpenAI provider requires apiKey')
  return createOpenAI({ apiKey: config.apiKey })(config.model)
}

function createGoogleModel(config: ResolvedLLMConfig): LanguageModel {
  if (!config.apiKey) throw new Error('Google provider requires apiKey')
  return createGoogleGenerativeAI({ apiKey: config.apiKey })(config.model)
}

function createOpenRouterModel(config: ResolvedLLMConfig): LanguageModel {
  if (!config.apiKey) throw new Error('OpenRouter provider requires apiKey')
  return createOpenRouter({
    apiKey: config.apiKey,
    extraBody: { reasoning: {} },
    fetch: createOpenRouterCompatibleFetch(),
  })(config.model)
}

function createAzureModel(config: ResolvedLLMConfig): LanguageModel {
  if (!config.apiKey || !config.resourceName) {
    throw new Error('Azure provider requires apiKey and resourceName')
  }
  return createAzure({
    resourceName: config.resourceName,
    apiKey: config.apiKey,
  })(config.model)
}

function createOllamaModel(config: ResolvedLLMConfig): LanguageModel {
  if (!config.baseUrl) throw new Error('Ollama provider requires baseUrl')
  return createOpenAICompatible({
    name: 'ollama',
    baseURL: config.baseUrl,
    ...(config.apiKey && { apiKey: config.apiKey }),
  })(config.model)
}

function createLMStudioModel(config: ResolvedLLMConfig): LanguageModel {
  if (!config.baseUrl) throw new Error('LMStudio provider requires baseUrl')
  return createOpenAICompatible({
    name: 'lmstudio',
    baseURL: config.baseUrl,
    ...(config.apiKey && { apiKey: config.apiKey }),
  })(config.model)
}

function createBedrockModel(config: ResolvedLLMConfig): LanguageModel {
  if (!config.accessKeyId || !config.secretAccessKey || !config.region) {
    throw new Error(
      'Bedrock provider requires accessKeyId, secretAccessKey, and region',
    )
  }
  return createAmazonBedrock({
    region: config.region,
    accessKeyId: config.accessKeyId,
    secretAccessKey: config.secretAccessKey,
    sessionToken: config.sessionToken,
  })(config.model)
}

function createBrowserOSModel(config: ResolvedLLMConfig): LanguageModel {
  if (!config.baseUrl) throw new Error('BrowserOS provider requires baseUrl')
  const { baseUrl, apiKey, model, upstreamProvider, browserosId } = config
  const browserosFetch = browserosId
    ? createBrowserOSFetch(browserosId)
    : createOpenRouterCompatibleFetch()

  if (upstreamProvider === LLM_PROVIDERS.OPENROUTER) {
    return createOpenRouter({
      baseURL: baseUrl,
      ...(apiKey && { apiKey }),
      fetch: browserosFetch,
    })(model)
  }
  if (upstreamProvider === LLM_PROVIDERS.ANTHROPIC) {
    return createAnthropic({
      baseURL: baseUrl,
      ...(apiKey && { apiKey }),
      fetch: browserosFetch,
    })(model)
  }
  if (upstreamProvider === LLM_PROVIDERS.AZURE) {
    return createAzure({
      baseURL: baseUrl,
      ...(apiKey && { apiKey }),
      fetch: browserosFetch,
    })(model)
  }
  logger.debug('Creating OpenAI-compatible provider for BrowserOS')
  return createOpenAICompatible({
    name: 'browseros',
    baseURL: baseUrl,
    ...(apiKey && { apiKey }),
    fetch: browserosFetch,
  })(model)
}

function createOpenAICompatibleModel(config: ResolvedLLMConfig): LanguageModel {
  if (!config.baseUrl)
    throw new Error('OpenAI-compatible provider requires baseUrl')
  return createOpenAICompatible({
    name: 'openai-compatible',
    baseURL: config.baseUrl,
    ...(config.apiKey && { apiKey: config.apiKey }),
  })(config.model)
}

function createMoonshotModel(config: ResolvedLLMConfig): LanguageModel {
  if (!config.baseUrl) throw new Error('Moonshot provider requires baseUrl')
  if (!config.apiKey) throw new Error('Moonshot provider requires apiKey')
  return createOpenAICompatible({
    name: 'moonshot',
    baseURL: config.baseUrl,
    apiKey: config.apiKey,
  })(config.model)
}

function createMinimaxModel(config: ResolvedLLMConfig): LanguageModel {
  if (!config.baseUrl) throw new Error('Minimax provider requires baseUrl')
  if (!config.apiKey) throw new Error('Minimax provider requires apiKey')
  return createOpenAICompatible({
    name: 'minimax',
    baseURL: config.baseUrl,
    apiKey: config.apiKey,
  })(config.model)
}

function createQwenCodeModel(config: ResolvedLLMConfig): LanguageModel {
  if (!config.apiKey) throw new Error('Qwen Code requires OAuth authentication')
  return createOpenAICompatible({
    name: 'qwen-code',
    baseURL: EXTERNAL_URLS.QWEN_CODE_API,
    apiKey: config.apiKey,
  })(config.model)
}

function createGitHubCopilotModel(config: ResolvedLLMConfig): LanguageModel {
  if (!config.apiKey)
    throw new Error('GitHub Copilot requires OAuth authentication')
  return createOpenAICompatible({
    name: 'github-copilot',
    baseURL: EXTERNAL_URLS.GITHUB_COPILOT_API,
    apiKey: config.apiKey,
    fetch: createCopilotFetch() as typeof globalThis.fetch,
  })(config.model)
}

function createChatGPTProModel(config: ResolvedLLMConfig): LanguageModel {
  if (!config.apiKey)
    throw new Error('ChatGPT Plus/Pro requires OAuth authentication')
  return createOpenAI({
    apiKey: config.apiKey,
    fetch: createCodexFetch(config.accountId) as typeof globalThis.fetch,
  }).responses(config.model)
}

const PROVIDER_FACTORIES: Record<string, ProviderFactory> = {
  [LLM_PROVIDERS.ANTHROPIC]: createAnthropicModel,
  [LLM_PROVIDERS.OPENAI]: createOpenAIModel,
  [LLM_PROVIDERS.GOOGLE]: createGoogleModel,
  [LLM_PROVIDERS.OPENROUTER]: createOpenRouterModel,
  [LLM_PROVIDERS.AZURE]: createAzureModel,
  [LLM_PROVIDERS.OLLAMA]: createOllamaModel,
  [LLM_PROVIDERS.LMSTUDIO]: createLMStudioModel,
  [LLM_PROVIDERS.BEDROCK]: createBedrockModel,
  [LLM_PROVIDERS.BROWSEROS]: createBrowserOSModel,
  [LLM_PROVIDERS.OPENAI_COMPATIBLE]: createOpenAICompatibleModel,
  [LLM_PROVIDERS.MOONSHOT]: createMoonshotModel,
  [LLM_PROVIDERS.CHATGPT_PRO]: createChatGPTProModel,
  [LLM_PROVIDERS.GITHUB_COPILOT]: createGitHubCopilotModel,
  [LLM_PROVIDERS.QWEN_CODE]: createQwenCodeModel,
  [LLM_PROVIDERS.MINIMAX]: createMinimaxModel,
}

export function createLLMProvider(config: ResolvedLLMConfig): LanguageModel {
  const factory = PROVIDER_FACTORIES[config.provider]
  if (!factory) throw new Error(`Unknown provider: ${config.provider}`)
  return factory(config)
}
