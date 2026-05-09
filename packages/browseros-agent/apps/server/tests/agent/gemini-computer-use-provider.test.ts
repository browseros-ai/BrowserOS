/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import assert from 'node:assert'
import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'
import { generateText, type LanguageModel } from 'ai'

import { createLanguageModel } from '../../src/agent/provider-factory'
import {
  addGeminiComputerUseTool,
  isGeminiComputerUseModel,
} from '../../src/lib/clients/llm/gemini-computer-use-fetch'
import { createLLMProvider } from '../../src/lib/clients/llm/provider'

const COMPUTER_USE_MODEL = 'gemini-2.5-computer-use-preview-10-2025'

async function captureGoogleRequest(model: LanguageModel) {
  const originalFetch = globalThis.fetch
  let capturedInput: RequestInfo | URL | undefined
  let capturedBody: unknown

  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    capturedInput = input
    capturedBody = JSON.parse(String(init?.body))

    return new Response(
      JSON.stringify({
        candidates: [
          {
            content: { parts: [{ text: 'ok' }] },
            finishReason: 'STOP',
          },
        ],
        usageMetadata: {
          promptTokenCount: 1,
          candidatesTokenCount: 1,
          totalTokenCount: 2,
        },
      }),
      {
        headers: { 'content-type': 'application/json' },
      },
    )
  }) as typeof fetch

  try {
    await generateText({
      model,
      messages: [{ role: 'user', content: 'Hello' }],
    })
  } finally {
    globalThis.fetch = originalFetch
  }

  assert.ok(capturedInput)
  assert.ok(capturedBody)
  return {
    url: String(capturedInput),
    body: capturedBody as { tools?: unknown[] },
  }
}

describe('Gemini Computer Use provider requests', () => {
  it('detects Gemini Computer Use model ids', () => {
    expect(isGeminiComputerUseModel(COMPUTER_USE_MODEL)).toBe(true)
    expect(isGeminiComputerUseModel('gemini-2.5-pro')).toBe(false)
  })

  it('adds Computer Use while preserving function declarations', () => {
    const body = addGeminiComputerUseTool({
      tools: [
        {
          functionDeclarations: [
            {
              name: 'custom_action',
              description: 'Custom action',
              parameters: { type: 'object' },
            },
          ],
        },
      ],
    }) as { tools: unknown[] }

    expect(body.tools).toEqual([
      {
        computerUse: {
          environment: 'ENVIRONMENT_BROWSER',
        },
      },
      {
        functionDeclarations: [
          {
            name: 'custom_action',
            description: 'Custom action',
            parameters: { type: 'object' },
          },
        ],
      },
    ])
  })

  it('injects Computer Use for agent provider factory requests', async () => {
    const model = createLanguageModel({
      conversationId: 'test-conversation',
      provider: LLM_PROVIDERS.GOOGLE,
      model: COMPUTER_USE_MODEL,
      apiKey: 'test-key',
      baseUrl: 'https://proxy.example.test/v1beta',
    })

    const request = await captureGoogleRequest(model)

    expect(request.url).toBe(
      `https://proxy.example.test/v1beta/models/${COMPUTER_USE_MODEL}:generateContent`,
    )
    expect(request.body.tools?.[0]).toEqual({
      computerUse: {
        environment: 'ENVIRONMENT_BROWSER',
      },
    })
  })

  it('injects Computer Use for lightweight LLM provider requests', async () => {
    const model = createLLMProvider({
      provider: LLM_PROVIDERS.GOOGLE,
      model: COMPUTER_USE_MODEL,
      apiKey: 'test-key',
    })

    const request = await captureGoogleRequest(model)

    expect(request.body.tools?.[0]).toEqual({
      computerUse: {
        environment: 'ENVIRONMENT_BROWSER',
      },
    })
  })

  it('leaves regular Gemini model requests unchanged', async () => {
    const model = createLanguageModel({
      conversationId: 'test-conversation',
      provider: LLM_PROVIDERS.GOOGLE,
      model: 'gemini-2.5-pro',
      apiKey: 'test-key',
    })

    const request = await captureGoogleRequest(model)

    expect(request.body.tools).toBeUndefined()
  })
})
