/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, describe, expect, it } from 'bun:test'
import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'
import { generateText } from 'ai'
import { createLLMProvider } from '../../../../src/lib/clients/llm/provider'

const originalFetch = globalThis.fetch

afterEach(() => {
  globalThis.fetch = originalFetch
})

function requestUrl(input: RequestInfo | URL): string {
  if (typeof input === 'string') return input
  if (input instanceof URL) return input.href
  return input.url
}

function responseBody(protocol: 'anthropic' | 'openai', model: string) {
  if (protocol === 'anthropic') {
    return {
      id: 'msg_test',
      type: 'message',
      role: 'assistant',
      content: [{ type: 'text', text: 'Hello' }],
      model,
      stop_reason: 'end_turn',
      stop_sequence: null,
      usage: { input_tokens: 1, output_tokens: 1 },
    }
  }

  return {
    id: 'chatcmpl_test',
    object: 'chat.completion',
    created: 0,
    model,
    choices: [
      {
        index: 0,
        message: { role: 'assistant', content: 'Hello' },
        finish_reason: 'stop',
      },
    ],
    usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
  }
}

const endpointCases = [
  {
    name: 'global OpenAI',
    baseUrl: 'https://api.minimax.io/v1',
    requestUrl: 'https://api.minimax.io/v1/chat/completions',
    protocol: 'openai',
    model: 'MiniMax-M3',
  },
  {
    name: 'China OpenAI',
    baseUrl: 'https://api.minimaxi.com/v1',
    requestUrl: 'https://api.minimaxi.com/v1/chat/completions',
    protocol: 'openai',
    model: 'MiniMax-M2.7',
  },
  {
    name: 'global Anthropic',
    baseUrl: 'https://api.minimax.io/anthropic',
    requestUrl: 'https://api.minimax.io/anthropic/v1/messages',
    protocol: 'anthropic',
    model: 'MiniMax-M3',
  },
  {
    name: 'China Anthropic',
    baseUrl: 'https://api.minimaxi.com/anthropic',
    requestUrl: 'https://api.minimaxi.com/anthropic/v1/messages',
    protocol: 'anthropic',
    model: 'MiniMax-M2.7',
  },
] as const

describe('createLLMProvider', () => {
  it('creates an OpenAI-compatible model for MiniMax', () => {
    const model = createLLMProvider({
      provider: LLM_PROVIDERS.MINIMAX,
      model: 'MiniMax-M3',
      apiKey: 'test-key',
      baseUrl: 'https://api.minimax.io/v1',
    })

    expect(model.provider).toBe('minimax.chat')
    expect(model.modelId).toBe('MiniMax-M3')
  })

  it('throws when the MiniMax provider is missing an apiKey', () => {
    expect(() =>
      createLLMProvider({
        provider: LLM_PROVIDERS.MINIMAX,
        model: 'MiniMax-M3',
        baseUrl: 'https://api.minimax.io/v1',
      }),
    ).toThrow('MiniMax provider requires apiKey')
  })

  for (const endpoint of endpointCases) {
    it(`sends requests to the ${endpoint.name} endpoint`, async () => {
      let capturedUrl = ''
      globalThis.fetch = (async (input: RequestInfo | URL) => {
        capturedUrl = requestUrl(input)
        return Response.json(responseBody(endpoint.protocol, endpoint.model))
      }) as typeof globalThis.fetch

      const model = createLLMProvider({
        provider: LLM_PROVIDERS.MINIMAX,
        model: endpoint.model,
        apiKey: 'test-key',
        baseUrl: endpoint.baseUrl,
      })
      const result = await generateText({ model, prompt: 'Hello' })

      expect(result.text).toBe('Hello')
      expect(capturedUrl).toBe(endpoint.requestUrl)
    })
  }
})
