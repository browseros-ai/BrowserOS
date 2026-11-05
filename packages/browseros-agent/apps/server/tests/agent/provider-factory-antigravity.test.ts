/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Antigravity is served by a local OpenAI-compatible bridge, so both provider
 * pipelines must route it through @ai-sdk/openai-compatible with the
 * configured baseUrl, an optional API key, and forwarded headers.
 */

import { beforeEach, describe, expect, it, mock } from 'bun:test'
import { LLM_PROVIDERS, LLMProviderSchema } from '@browseros/shared/schemas/llm'

let lastArgs: Record<string, unknown> | null = null
let lastModelId: string | null = null

mock.module('@ai-sdk/openai-compatible', () => ({
  createOpenAICompatible: (args: Record<string, unknown>) => {
    lastArgs = args
    return (modelId: string) => {
      lastModelId = modelId
      return { modelId }
    }
  },
}))

const { createLanguageModel, serializesReasoningContent } = await import(
  '../../src/agent/provider-factory'
)
const { createLLMProvider } = await import('../../src/lib/clients/llm/provider')

const BRIDGE_URL = 'http://127.0.0.1:8765/v1'

beforeEach(() => {
  lastArgs = null
  lastModelId = null
})

describe('antigravity provider', () => {
  it('is a valid LLM provider id', () => {
    expect(LLM_PROVIDERS.ANTIGRAVITY).toBe('antigravity')
    expect(LLMProviderSchema.parse('antigravity')).toBe('antigravity')
  })

  it('agent factory points the openai-compatible SDK at the bridge', async () => {
    await createLanguageModel({
      conversationId: 'c1',
      provider: LLM_PROVIDERS.ANTIGRAVITY,
      model: 'gemini-3.8-flash',
      baseUrl: BRIDGE_URL,
      headers: { 'x-static': 'value' },
    })
    expect(lastArgs).toEqual({
      headers: { 'x-static': 'value' },
      name: 'antigravity',
      baseURL: BRIDGE_URL,
    })
    expect(lastModelId).toBe('gemini-3.8-flash')
  })

  it('forwards an optional bridge token as apiKey', () => {
    createLLMProvider({
      provider: LLM_PROVIDERS.ANTIGRAVITY,
      model: 'gemini-3.8-flash',
      baseUrl: BRIDGE_URL,
      apiKey: 'bridge-token',
    })
    expect(lastArgs).toMatchObject({
      name: 'antigravity',
      baseURL: BRIDGE_URL,
      apiKey: 'bridge-token',
    })
    expect(lastModelId).toBe('gemini-3.8-flash')
  })

  it('requires a baseUrl in both pipelines', async () => {
    expect(() =>
      createLLMProvider({
        provider: LLM_PROVIDERS.ANTIGRAVITY,
        model: 'gemini-3.8-flash',
      }),
    ).toThrow(/Antigravity provider requires baseUrl/)
    await expect(
      createLanguageModel({
        conversationId: 'c1',
        provider: LLM_PROVIDERS.ANTIGRAVITY,
        model: 'gemini-3.8-flash',
      }),
    ).rejects.toThrow(/Antigravity provider requires baseUrl/)
  })

  it('strips replayed reasoning like other openai-compatible providers', () => {
    expect(serializesReasoningContent(LLM_PROVIDERS.ANTIGRAVITY)).toBe(true)
  })
})
