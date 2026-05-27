import { describe, expect, it, mock } from 'bun:test'

import type { ResolvedAgentConfig } from '../../src/agent/types'

const LLM_PROVIDERS = {
  ANTHROPIC: 'anthropic',
  OPENAI: 'openai',
  GOOGLE: 'google',
  OPENROUTER: 'openrouter',
  AZURE: 'azure',
  OLLAMA: 'ollama',
  LMSTUDIO: 'lmstudio',
  BEDROCK: 'bedrock',
  BROWSEROS: 'browseros',
  OPENAI_COMPATIBLE: 'openai-compatible',
  MOONSHOT: 'moonshot',
  CHATGPT_PRO: 'chatgpt-pro',
  GITHUB_COPILOT: 'github-copilot',
  QWEN_CODE: 'qwen-code',
} as const

type OpenAICompatibleOptions = {
  name: string
  baseURL: string
  apiKey?: string
  fetch?: typeof globalThis.fetch
}

const openAICompatibleOptions: OpenAICompatibleOptions[] = []
const createProviderFactory = () =>
  mock((options: unknown) =>
    mock((modelId: string) => ({
      modelId,
      options,
    })),
  )

mock.module('@browseros/shared/schemas/llm', () => ({
  LLM_PROVIDERS,
}))

mock.module('@browseros/shared/constants/urls', () => ({
  EXTERNAL_URLS: {
    QWEN_CODE_API: 'https://qwen.example.test',
    GITHUB_COPILOT_API: 'https://copilot.example.test',
  },
}))

mock.module('@ai-sdk/amazon-bedrock', () => ({
  createAmazonBedrock: createProviderFactory(),
}))

mock.module('@ai-sdk/anthropic', () => ({
  createAnthropic: createProviderFactory(),
}))

mock.module('@ai-sdk/azure', () => ({
  createAzure: createProviderFactory(),
}))

mock.module('@ai-sdk/google', () => ({
  createGoogleGenerativeAI: createProviderFactory(),
}))

mock.module('@ai-sdk/openai', () => ({
  createOpenAI: mock((options: unknown) => ({
    ...createProviderFactory()(options),
    responses: createProviderFactory()(options),
  })),
}))

mock.module('@ai-sdk/openai-compatible', () => ({
  createOpenAICompatible: mock((options: OpenAICompatibleOptions) => {
    openAICompatibleOptions.push(options)
    return mock((modelId: string) => ({
      modelId,
      options,
    }))
  }),
}))

mock.module('@openrouter/ai-sdk-provider', () => ({
  createOpenRouter: createProviderFactory(),
}))

mock.module('../../src/lib/browseros-fetch', () => ({
  createBrowserOSFetch: () => globalThis.fetch,
}))

mock.module('../../src/lib/clients/llm/mock-language-model', () => ({
  createMockBrowserOSLanguageModel: mock(() => ({})),
  shouldUseMockBrowserOSLLM: mock(() => false),
}))

mock.module('../../src/lib/clients/oauth/codex-fetch', () => ({
  createCodexFetch: () => globalThis.fetch,
}))

mock.module('../../src/lib/clients/oauth/copilot-fetch', () => ({
  createCopilotFetch: () => globalThis.fetch,
}))

mock.module('../../src/lib/logger', () => ({
  logger: {
    debug: mock(() => {}),
  },
}))

mock.module('../../src/lib/openrouter-fetch', () => ({
  createOpenRouterCompatibleFetch: () => globalThis.fetch,
}))

const { createLanguageModel } = await import('../../src/agent/provider-factory')

function createConfig(
  provider: ResolvedAgentConfig['provider'],
  requestTimeoutMs?: number,
): ResolvedAgentConfig {
  return {
    conversationId: crypto.randomUUID(),
    provider,
    model: 'local-model',
    baseUrl: 'http://localhost:11434/v1',
    requestTimeoutMs,
  }
}

async function withFetch<T>(
  fetchImpl: typeof globalThis.fetch,
  callback: () => Promise<T>,
): Promise<T> {
  const originalFetch = globalThis.fetch
  globalThis.fetch = fetchImpl
  try {
    return await callback()
  } finally {
    globalThis.fetch = originalFetch
  }
}

function getLastOpenAICompatibleOptions(): OpenAICompatibleOptions {
  const options = openAICompatibleOptions.at(-1)
  if (!options) throw new Error('Expected createOpenAICompatible to be called')
  return options
}

describe('createLanguageModel local provider request timeouts', () => {
  it('does not pass a fetch override when timeout is unset', () => {
    createLanguageModel(createConfig(LLM_PROVIDERS.OLLAMA))

    expect(getLastOpenAICompatibleOptions().fetch).toBeUndefined()
  })

  it('does not pass a fetch override when timeout is zero or negative', () => {
    createLanguageModel(createConfig(LLM_PROVIDERS.OLLAMA, 0))
    expect(getLastOpenAICompatibleOptions().fetch).toBeUndefined()

    createLanguageModel(createConfig(LLM_PROVIDERS.LMSTUDIO, -1))
    expect(getLastOpenAICompatibleOptions().fetch).toBeUndefined()
  })

  it.each([
    [LLM_PROVIDERS.OLLAMA, 'ollama'],
    [LLM_PROVIDERS.LMSTUDIO, 'lmstudio'],
  ] as const)(
    'passes through fast %s requests before the timeout',
    async (provider, providerName) => {
      createLanguageModel(createConfig(provider, 100))
      const timeoutFetch = getLastOpenAICompatibleOptions().fetch

      expect(timeoutFetch).toBeDefined()
      await withFetch(
        mock(async (_input, init) => {
          expect(init?.signal).toBeDefined()
          expect(init?.signal?.aborted).toBe(false)
          return new Response(providerName)
        }) as typeof globalThis.fetch,
        async () => {
          const response = await timeoutFetch?.('http://localhost/v1/chat')
          expect(await response?.text()).toBe(providerName)
        },
      )
    },
  )

  it.each([
    [LLM_PROVIDERS.OLLAMA],
    [LLM_PROVIDERS.LMSTUDIO],
  ] as const)('aborts slow %s requests after the timeout', async (provider) => {
    createLanguageModel(createConfig(provider, 10))
    const timeoutFetch = getLastOpenAICompatibleOptions().fetch

    expect(timeoutFetch).toBeDefined()
    await withFetch(
      mock(
        async (
          _input: RequestInfo | URL,
          init?: RequestInit,
        ): Promise<Response> => {
          await new Promise((resolve, reject) => {
            init?.signal?.addEventListener(
              'abort',
              () => reject(init.signal?.reason),
              { once: true },
            )
          })
          return new Response('unexpected')
        },
      ) as typeof globalThis.fetch,
      async () => {
        await expect(timeoutFetch?.('http://localhost/v1/chat')).rejects.toThrow(
          /abort|timed out/i,
        )
      },
    )
  })
})
