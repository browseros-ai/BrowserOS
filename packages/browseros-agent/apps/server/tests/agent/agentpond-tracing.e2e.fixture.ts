/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { mock } from 'bun:test'
import { MockLanguageModelV3 } from 'ai/test'

const PROMPT_SENTINEL = 'browseros-agentpond-private-prompt'
const RESPONSE_SENTINEL = 'browseros-agentpond-private-response'

const model = new MockLanguageModelV3({
  provider: 'browseros-agentpond-e2e',
  modelId: 'browseros-agentpond-test-model',
  doGenerate: {
    content: [{ type: 'text', text: RESPONSE_SENTINEL }],
    finishReason: { raw: 'stop', unified: 'stop' },
    usage: {
      inputTokens: { cacheRead: 0, cacheWrite: 0, noCache: 7, total: 7 },
      outputTokens: { reasoning: 0, text: 4, total: 4 },
    },
    warnings: [],
  },
})

mock.module('../../src/agent/provider-factory', () => ({
  createLanguageModel: async () => ({ model }),
}))

const { AiSdkAgent } = await import('../../src/agent/ai-sdk-agent')
const { flushAgentPondTracing, shutdownAgentPondTracing } = await import(
  '../../src/agent/agentpond-tracing'
)

const agent = await AiSdkAgent.create({
  browserSession: {} as never,
  resolvedConfig: {
    apiKey: 'fixture',
    baseUrl: 'http://127.0.0.1',
    chatMode: true,
    conversationId: 'agentpond-e2e',
    model: 'browseros-agentpond-test-model',
    provider: 'openai-compatible',
  },
})

try {
  const result = await agent.toolLoopAgent.generate({
    prompt: PROMPT_SENTINEL,
  })
  if (result.text !== RESPONSE_SENTINEL) {
    throw new Error('AgentPond E2E fixture returned an unexpected response')
  }
  if (result.totalUsage.totalTokens !== 11) {
    throw new Error('AgentPond E2E fixture returned unexpected token usage')
  }
  await flushAgentPondTracing()
} finally {
  await agent.dispose()
  await shutdownAgentPondTracing()
  mock.restore()
}
