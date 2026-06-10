import type {
  LanguageModelV3GenerateResult,
  LanguageModelV3StreamPart,
  LanguageModelV3Usage,
} from '@ai-sdk/provider'
import { LLM_PROVIDERS, type LLMConfig } from '@browseros/shared/schemas/llm'
import { type LanguageModel, simulateReadableStream } from 'ai'
import { MockLanguageModelV3 } from 'ai/test'
import type { ResolvedLLMConfig } from './types'

const MOCK_BROWSEROS_MODEL_ID = 'browseros-test-mock'
export const MOCK_BROWSEROS_RESPONSE_TEXT = 'Mock BrowserOS test response.'

const MOCK_USAGE: LanguageModelV3Usage = {
  inputTokens: {
    total: 1,
    noCache: 1,
    cacheRead: undefined,
    cacheWrite: undefined,
  },
  outputTokens: {
    total: 4,
    text: 4,
    reasoning: undefined,
  },
}

function createMockResult(): LanguageModelV3GenerateResult {
  return {
    content: [{ type: 'text', text: MOCK_BROWSEROS_RESPONSE_TEXT }],
    finishReason: { unified: 'stop', raw: 'stop' },
    usage: MOCK_USAGE,
    warnings: [],
  }
}

function isMockBrowserOSLLMEnabled(): boolean {
  return process.env.BROWSEROS_USE_MOCK_LLM === 'true'
}

export function shouldUseMockBrowserOSLLM(
  config: Pick<LLMConfig, 'provider'>,
): boolean {
  return (
    config.provider === LLM_PROVIDERS.BROWSEROS && isMockBrowserOSLLMEnabled()
  )
}

export function resolveMockBrowserOSConfig(
  config: LLMConfig,
  browserosId?: string,
): ResolvedLLMConfig {
  return {
    ...config,
    model: config.model ?? MOCK_BROWSEROS_MODEL_ID,
    browserosId,
    upstreamProvider: LLM_PROVIDERS.OPENAI,
  }
}

export function createMockBrowserOSLanguageModel(): LanguageModel {
  const chunks: LanguageModelV3StreamPart[] = [
    { type: 'text-start', id: 'text-1' },
    {
      type: 'text-delta',
      id: 'text-1',
      delta: MOCK_BROWSEROS_RESPONSE_TEXT,
    },
    { type: 'text-end', id: 'text-1' },
    {
      type: 'finish',
      finishReason: { unified: 'stop', raw: 'stop' },
      usage: MOCK_USAGE,
    },
  ]

  return new MockLanguageModelV3({
    doGenerate: async () => createMockResult(),
    doStream: async () => ({
      stream: simulateReadableStream({ chunks }),
    }),
  }) as LanguageModel
}
