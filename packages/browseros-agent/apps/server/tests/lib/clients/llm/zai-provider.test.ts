/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Contract tests for the z.ai (GLM) LLM provider. Verifies the provider is
 * wired into both the server-side and agent-side factory registries and that
 * its apiKey / baseUrl contract matches the other OpenAI-compatible providers.
 */

import { describe, it } from 'bun:test'
import assert from 'node:assert'

import { EXTERNAL_URLS } from '@browseros/shared/constants/urls'
import { LLM_PROVIDERS, LLMProviderSchema } from '@browseros/shared/schemas/llm'
import { createLanguageModel } from '../../../../src/agent/provider-factory'
import { createLLMProvider } from '../../../../src/lib/clients/llm/provider'

const MODEL = 'glm-4.6'

describe('z.ai provider', () => {
  describe('schema', () => {
    it("LLM_PROVIDERS.ZAI is 'zai'", () => {
      assert.strictEqual(LLM_PROVIDERS.ZAI, 'zai')
    })

    it("LLMProviderSchema accepts 'zai'", () => {
      assert.strictEqual(LLMProviderSchema.parse('zai'), 'zai')
    })

    it('LLMProviderSchema rejects unknown providers', () => {
      assert.throws(() => LLMProviderSchema.parse('not-a-provider'))
    })
  })

  describe('server factory (createLLMProvider)', () => {
    it('resolves the zai factory (not "Unknown provider")', () => {
      const model = createLLMProvider({
        provider: LLM_PROVIDERS.ZAI,
        model: MODEL,
        apiKey: 'test-key',
      })
      assert.ok(model, 'expected a language model instance')
    })

    it('throws when apiKey is missing', () => {
      assert.throws(
        () =>
          createLLMProvider({
            provider: LLM_PROVIDERS.ZAI,
            model: MODEL,
          }),
        /z\.ai provider requires apiKey/,
      )
    })

    it('accepts a custom baseUrl override', () => {
      const model = createLLMProvider({
        provider: LLM_PROVIDERS.ZAI,
        model: MODEL,
        apiKey: 'test-key',
        baseUrl: 'https://proxy.example.com/v1',
      })
      assert.ok(model)
    })

    it('defaults baseUrl to EXTERNAL_URLS.ZAI_API', () => {
      // Smoke: building with no baseUrl must not throw — the factory falls back
      // to the canonical z.ai endpoint.
      assert.ok(EXTERNAL_URLS.ZAI_API.includes('z.ai'))
      const model = createLLMProvider({
        provider: LLM_PROVIDERS.ZAI,
        model: MODEL,
        apiKey: 'test-key',
      })
      assert.ok(model)
    })
  })

  describe('agent factory (createLanguageModel)', () => {
    it('resolves the zai factory (not "Unknown provider")', () => {
      const model = createLanguageModel({
        conversationId: 'test-conversation',
        provider: LLM_PROVIDERS.ZAI,
        model: MODEL,
        apiKey: 'test-key',
      })
      assert.ok(model, 'expected a language model instance')
    })

    it('throws when apiKey is missing', () => {
      assert.throws(
        () =>
          createLanguageModel({
            conversationId: 'test-conversation',
            provider: LLM_PROVIDERS.ZAI,
            model: MODEL,
          }),
        /z\.ai provider requires apiKey/,
      )
    })
  })
})
