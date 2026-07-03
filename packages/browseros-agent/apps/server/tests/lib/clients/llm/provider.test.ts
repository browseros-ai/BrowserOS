/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'
import { createLLMProvider } from '../../../../src/lib/clients/llm/provider'

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
})
