import { mkdir } from 'node:fs/promises'
import { homedir } from 'node:os'
import { join } from 'node:path'
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
import { buildAcpxProvider } from '../lib/agents/acpx-provider/buildAcpxProvider'
import { getBrowserosDir } from '../lib/browseros-dir'
import { createBrowserOSFetch } from '../lib/browseros-fetch'
import {
  createMockBrowserOSLanguageModel,
  shouldUseMockBrowserOSLLM,
} from '../lib/clients/llm/mock-language-model'
import { createCodexFetch } from '../lib/clients/oauth/codex-fetch'
import { createCopilotFetch } from '../lib/clients/oauth/copilot-fetch'
import { logger } from '../lib/logger'
import { createOpenRouterCompatibleFetch } from '../lib/openrouter-fetch'
import { ACP_PROVIDER_TYPES, isAcpProvider } from './acp-providers'
import type { ResolvedAgentConfig } from './types'

export { isAcpProvider }

const BUILT_IN_ACP_AGENT_BY_PROVIDER: Record<string, string> = {
  [LLM_PROVIDERS.CLAUDE_CODE]: 'claude',
  [LLM_PROVIDERS.CODEX]: 'codex',
}

function defaultAcpWorkspacePath(providerId: string): string {
  return join(getBrowserosDir(), 'workspaces', providerId)
}

/**
 * Substitute a leading `$HOME` token with the actual home directory.
 * The harness-to-providers migration (follow-up PR) writes
 * `$HOME/browseros-workspaces/...` as a placeholder because the
 * renderer cannot read `$HOME` directly; node's `child_process.spawn`
 * does NOT expand shell variables in its `cwd` option, so we have to
 * substitute server-side before the path reaches the spawn boundary.
 */
function expandHomeToken(path: string): string {
  return path.replace(/^\$HOME(?=\/|$)/, homedir())
}

function resolveAcpAgentId(config: ResolvedAgentConfig): string {
  if (config.provider === LLM_PROVIDERS.ACP_CUSTOM) {
    if (!config.acpAgentId) {
      throw new Error('acp-custom provider requires acpAgentId')
    }
    return config.acpAgentId
  }
  const builtIn = BUILT_IN_ACP_AGENT_BY_PROVIDER[config.provider]
  if (!builtIn) {
    throw new Error(`Unknown ACP provider type: ${config.provider}`)
  }
  return config.acpAgentId ?? builtIn
}

async function createAcpLanguageModel(
  config: ResolvedAgentConfig,
): Promise<LanguageModelWithCleanup> {
  const agentId = resolveAcpAgentId(config)
  const workspacePath = expandHomeToken(
    config.acpFixedWorkspacePath ?? defaultAcpWorkspacePath(config.provider),
  )
  await mkdir(workspacePath, { recursive: true }).catch((err: unknown) => {
    logger.warn('Failed to ensure ACP workspace exists; spawn may fail', {
      workspacePath,
      error: err instanceof Error ? err.message : String(err),
    })
  })
  const agentRegistryOverrides: Record<string, string> = {}
  if (config.provider === LLM_PROVIDERS.ACP_CUSTOM && config.acpCommand) {
    agentRegistryOverrides[agentId] = config.acpCommand
  }
  const provider = await buildAcpxProvider({
    conversationId: config.conversationId,
    agentId,
    workspacePath,
    agentRegistryOverrides,
    mcpServers: config.acpMcpServers,
  })
  return {
    model: provider.languageModel() as LanguageModel,
    // acpx-ai-provider's docs put close() ownership on the caller: skip
    // it and the spawned agent process outlives the conversation.
    close: () => provider.close(),
  }
}

type ProviderFactory = (
  config: ResolvedAgentConfig,
) => (modelId: string) => unknown

function createAnthropicFactory(
  config: ResolvedAgentConfig,
): (modelId: string) => unknown {
  if (!config.apiKey) throw new Error('Anthropic provider requires apiKey')
  return createAnthropic({ apiKey: config.apiKey })
}

function createOpenAIFactory(
  config: ResolvedAgentConfig,
): (modelId: string) => unknown {
  if (!config.apiKey) throw new Error('OpenAI provider requires apiKey')
  return createOpenAI({ apiKey: config.apiKey })
}

function createGoogleFactory(
  config: ResolvedAgentConfig,
): (modelId: string) => unknown {
  if (!config.apiKey) throw new Error('Google provider requires apiKey')
  return createGoogleGenerativeAI({ apiKey: config.apiKey })
}

function createOpenRouterFactory(
  config: ResolvedAgentConfig,
): (modelId: string) => unknown {
  if (!config.apiKey) throw new Error('OpenRouter provider requires apiKey')
  return createOpenRouter({
    apiKey: config.apiKey,
    extraBody: { reasoning: {} },
    fetch: createOpenRouterCompatibleFetch(),
  })
}

function createAzureFactory(
  config: ResolvedAgentConfig,
): (modelId: string) => unknown {
  if (!config.apiKey || !config.resourceName) {
    throw new Error('Azure provider requires apiKey and resourceName')
  }
  return createAzure({
    resourceName: config.resourceName,
    apiKey: config.apiKey,
  })
}

function createLMStudioFactory(
  config: ResolvedAgentConfig,
): (modelId: string) => unknown {
  if (!config.baseUrl) throw new Error('LMStudio provider requires baseUrl')
  return createOpenAICompatible({
    name: 'lmstudio',
    baseURL: config.baseUrl,
    ...(config.apiKey && { apiKey: config.apiKey }),
  })
}

function createOllamaFactory(
  config: ResolvedAgentConfig,
): (modelId: string) => unknown {
  if (!config.baseUrl) throw new Error('Ollama provider requires baseUrl')
  return createOpenAICompatible({
    name: 'ollama',
    baseURL: config.baseUrl,
    ...(config.apiKey && { apiKey: config.apiKey }),
  })
}

function createBedrockFactory(
  config: ResolvedAgentConfig,
): (modelId: string) => unknown {
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
  })
}

function createBrowserOSFactory(
  config: ResolvedAgentConfig,
): (modelId: string) => unknown {
  if (!config.baseUrl) throw new Error('BrowserOS provider requires baseUrl')
  const { baseUrl, apiKey, upstreamProvider, browserosId } = config
  const browserosFetch = browserosId
    ? createBrowserOSFetch(browserosId)
    : createOpenRouterCompatibleFetch()

  if (upstreamProvider === LLM_PROVIDERS.OPENROUTER) {
    return createOpenRouter({
      baseURL: baseUrl,
      ...(apiKey && { apiKey }),
      fetch: browserosFetch,
    })
  }
  if (upstreamProvider === LLM_PROVIDERS.ANTHROPIC) {
    return createAnthropic({
      baseURL: baseUrl,
      ...(apiKey && { apiKey }),
      fetch: browserosFetch,
    })
  }
  if (upstreamProvider === LLM_PROVIDERS.AZURE) {
    return createAzure({
      baseURL: baseUrl,
      ...(apiKey && { apiKey }),
      fetch: browserosFetch,
    })
  }
  logger.debug('Creating OpenAI-compatible provider for BrowserOS')
  return createOpenAICompatible({
    name: 'browseros',
    baseURL: baseUrl,
    ...(apiKey && { apiKey }),
    fetch: browserosFetch,
  })
}

function createOpenAICompatibleFactory(
  config: ResolvedAgentConfig,
): (modelId: string) => unknown {
  if (!config.baseUrl)
    throw new Error('OpenAI-compatible provider requires baseUrl')
  return createOpenAICompatible({
    name: 'openai-compatible',
    baseURL: config.baseUrl,
    ...(config.apiKey && { apiKey: config.apiKey }),
  })
}

function createMoonshotFactory(
  config: ResolvedAgentConfig,
): (modelId: string) => unknown {
  if (!config.baseUrl) throw new Error('Moonshot provider requires baseUrl')
  if (!config.apiKey) throw new Error('Moonshot provider requires apiKey')
  return createOpenAICompatible({
    name: 'moonshot',
    baseURL: config.baseUrl,
    apiKey: config.apiKey,
  })
}

function createQwenCodeFactory(
  config: ResolvedAgentConfig,
): (modelId: string) => unknown {
  if (!config.apiKey) throw new Error('Qwen Code requires OAuth authentication')
  return createOpenAICompatible({
    name: 'qwen-code',
    baseURL: EXTERNAL_URLS.QWEN_CODE_API,
    apiKey: config.apiKey,
  })
}

function createGitHubCopilotFactory(
  config: ResolvedAgentConfig,
): (modelId: string) => unknown {
  if (!config.apiKey)
    throw new Error('GitHub Copilot requires OAuth authentication')
  return createOpenAICompatible({
    name: 'github-copilot',
    baseURL: EXTERNAL_URLS.GITHUB_COPILOT_API,
    apiKey: config.apiKey,
    fetch: createCopilotFetch() as typeof globalThis.fetch,
  })
}

function createChatGPTProFactory(
  config: ResolvedAgentConfig,
): (modelId: string) => unknown {
  if (!config.apiKey)
    throw new Error('ChatGPT Plus/Pro requires OAuth authentication')
  return createOpenAI({
    apiKey: config.apiKey,
    fetch: createCodexFetch(config.accountId) as typeof globalThis.fetch,
  }).responses
}

const PROVIDER_FACTORIES: Record<string, ProviderFactory> = {
  [LLM_PROVIDERS.ANTHROPIC]: createAnthropicFactory,
  [LLM_PROVIDERS.OPENAI]: createOpenAIFactory,
  [LLM_PROVIDERS.GOOGLE]: createGoogleFactory,
  [LLM_PROVIDERS.OPENROUTER]: createOpenRouterFactory,
  [LLM_PROVIDERS.AZURE]: createAzureFactory,
  [LLM_PROVIDERS.LMSTUDIO]: createLMStudioFactory,
  [LLM_PROVIDERS.OLLAMA]: createOllamaFactory,
  [LLM_PROVIDERS.BEDROCK]: createBedrockFactory,
  [LLM_PROVIDERS.BROWSEROS]: createBrowserOSFactory,
  [LLM_PROVIDERS.OPENAI_COMPATIBLE]: createOpenAICompatibleFactory,
  [LLM_PROVIDERS.MOONSHOT]: createMoonshotFactory,
  [LLM_PROVIDERS.CHATGPT_PRO]: createChatGPTProFactory,
  [LLM_PROVIDERS.GITHUB_COPILOT]: createGitHubCopilotFactory,
  [LLM_PROVIDERS.QWEN_CODE]: createQwenCodeFactory,
}

export interface LanguageModelWithCleanup {
  model: LanguageModel
  /**
   * Caller-owned teardown. Only set for providers that own a spawned
   * process or persistent session (today: ACP providers via
   * `acpx-ai-provider`); model-backed factories leave it undefined.
   * `AiSdkAgent.dispose()` awaits this so the agent process exits with
   * the conversation.
   */
  close?: () => Promise<void>
}

export async function createLanguageModel(
  config: ResolvedAgentConfig,
): Promise<LanguageModelWithCleanup> {
  if (shouldUseMockBrowserOSLLM(config)) {
    return { model: createMockBrowserOSLanguageModel() }
  }
  const provider = config.provider as string
  if (ACP_PROVIDER_TYPES.has(provider)) {
    return createAcpLanguageModel(config)
  }
  const factory = PROVIDER_FACTORIES[provider]
  if (!factory) throw new Error(`Unknown provider: ${provider}`)
  return { model: factory(config)(config.model) as LanguageModel }
}
