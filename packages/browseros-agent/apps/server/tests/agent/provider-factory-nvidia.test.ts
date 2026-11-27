/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Regression coverage: the NVIDIA NIM factories in both provider
 * pipelines must build an OpenAI-compatible provider named `nvidia`
 * with the configured baseUrl and apiKey, forward custom headers, and
 * reject configs missing either field.
 */

import { beforeEach, describe, expect, it, mock } from 'bun:test'
import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'

let lastCallArgs: Record<string, unknown> | null = null

mock.module('@ai-sdk/openai-compatible', () => ({
  createOpenAICompatible: (args: Record<string, unknown>) => {
    lastCallArgs = args
    return (modelId: string) => ({ modelId })
  },
}))

mock.module('../../src/lib/openrouter-fetch', () => ({
  createOpenRouterCompatibleFetch: () => globalThis.fetch,
}))

const { createLanguageModel } = await import('../../src/agent/provider-factory')
const { createLLMProvider } = await import('../../src/lib/clients/llm/provider')

beforeEach(() => {
  lastCallArgs = null
})

const NVIDIA_BASE_URL = 'https://integrate.api.nvidia.com/v1'
const MODEL = 'nvidia/nemotron-3-super-120b-a12b'

describe('createNvidiaFactory', () => {
  it('builds an OpenAI-compatible provider named nvidia', async () => {
    const { model } = await createLanguageModel({
      conversationId: 'c1',
      provider: LLM_PROVIDERS.NVIDIA,
      model: MODEL,
      apiKey: 'nv-key',
      baseUrl: NVIDIA_BASE_URL,
    })
    expect(lastCallArgs).toEqual({
      name: 'nvidia',
      baseURL: NVIDIA_BASE_URL,
      apiKey: 'nv-key',
    })
    expect(model).toEqual({ modelId: MODEL } as unknown as typeof model)
  })

  it('forwards a custom baseUrl for self-hosted NIM', async () => {
    await createLanguageModel({
      conversationId: 'c1',
      provider: LLM_PROVIDERS.NVIDIA,
      model: MODEL,
      apiKey: 'nv-key',
      baseUrl: 'http://nim.internal:8000/v1',
    })
    expect(lastCallArgs).toMatchObject({
      name: 'nvidia',
      baseURL: 'http://nim.internal:8000/v1',
    })
  })

  it('forwards resolved headers', async () => {
    await createLanguageModel({
      conversationId: 'conversation-1',
      provider: LLM_PROVIDERS.NVIDIA,
      model: MODEL,
      apiKey: 'nv-key',
      baseUrl: NVIDIA_BASE_URL,
      headers: {
        'x-session': 'chat-{{conversationId}}',
        'x-static': 'value',
      },
    })
    expect(lastCallArgs?.headers).toEqual({
      'x-session': 'chat-conversation-1',
      'x-static': 'value',
    })
  })

  it('throws when baseUrl is missing', async () => {
    await expect(
      createLanguageModel({
        conversationId: 'c1',
        provider: LLM_PROVIDERS.NVIDIA,
        model: MODEL,
        apiKey: 'nv-key',
      }),
    ).rejects.toThrow('NVIDIA provider requires baseUrl')
  })

  it('throws when apiKey is missing', async () => {
    await expect(
      createLanguageModel({
        conversationId: 'c1',
        provider: LLM_PROVIDERS.NVIDIA,
        model: MODEL,
        baseUrl: NVIDIA_BASE_URL,
      }),
    ).rejects.toThrow('NVIDIA provider requires apiKey')
  })
})

describe('createNvidiaModel', () => {
  it('builds an OpenAI-compatible provider named nvidia', () => {
    createLLMProvider({
      provider: LLM_PROVIDERS.NVIDIA,
      model: MODEL,
      apiKey: 'nv-key',
      baseUrl: NVIDIA_BASE_URL,
      headers: { 'x-static': 'value' },
    })
    expect(lastCallArgs).toMatchObject({
      name: 'nvidia',
      baseURL: NVIDIA_BASE_URL,
      apiKey: 'nv-key',
      headers: { 'x-static': 'value' },
    })
  })

  it('throws when baseUrl is missing', () => {
    expect(() =>
      createLLMProvider({
        provider: LLM_PROVIDERS.NVIDIA,
        model: MODEL,
        apiKey: 'nv-key',
      }),
    ).toThrow('NVIDIA provider requires baseUrl')
  })

  it('throws when apiKey is missing', () => {
    expect(() =>
      createLLMProvider({
        provider: LLM_PROVIDERS.NVIDIA,
        model: MODEL,
        baseUrl: NVIDIA_BASE_URL,
      }),
    ).toThrow('NVIDIA provider requires apiKey')
  })
})
