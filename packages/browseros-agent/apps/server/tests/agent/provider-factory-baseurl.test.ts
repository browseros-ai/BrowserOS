/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Regression coverage: every URL-configurable model-backed factory
 * (Anthropic, OpenAI, Google, OpenRouter, Azure) must forward a
 * configured `baseUrl` as the SDK's `baseURL` option and omit it
 * when the field is unset so the SDK default endpoint stands.
 *
 * The OpenAI branch also carries a UI hint in NewProviderDialog
 * pointing users at the "OpenAI Compatible" provider template when
 * a custom endpoint speaks Chat Completions instead of the default
 * Responses API; that hint is a UX concern, not a factory concern.
 */

import { beforeEach, describe, expect, it, mock } from 'bun:test'
import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'

// Each SDK is mocked as a factory that records its args on
// `lastCallArgs` and returns a callable stand-in for the language
// model provider. Live in module scope so the mocks resolve before
// `provider-factory.ts` is imported below.
const lastCallArgs: {
  anthropic: Record<string, unknown> | null
  openai: Record<string, unknown> | null
  google: Record<string, unknown> | null
  openrouter: Record<string, unknown> | null
  azure: Record<string, unknown> | null
} = {
  anthropic: null,
  openai: null,
  google: null,
  openrouter: null,
  azure: null,
}

function fakeProvider(): (modelId: string) => unknown {
  const fn = (modelId: string) => ({ modelId })
  return fn
}

mock.module('@ai-sdk/anthropic', () => ({
  createAnthropic: (args: Record<string, unknown>) => {
    lastCallArgs.anthropic = args
    return fakeProvider()
  },
}))

mock.module('@ai-sdk/openai', () => ({
  createOpenAI: (args: Record<string, unknown>) => {
    lastCallArgs.openai = args
    return fakeProvider()
  },
}))

mock.module('@ai-sdk/google', () => ({
  createGoogleGenerativeAI: (args: Record<string, unknown>) => {
    lastCallArgs.google = args
    return fakeProvider()
  },
}))

mock.module('@openrouter/ai-sdk-provider', () => ({
  createOpenRouter: (args: Record<string, unknown>) => {
    lastCallArgs.openrouter = args
    return fakeProvider()
  },
}))

mock.module('@ai-sdk/azure', () => ({
  createAzure: (args: Record<string, unknown>) => {
    lastCallArgs.azure = args
    return fakeProvider()
  },
}))

// Stub the OpenRouter-compatible fetch helper so the factory does not
// try to reach the network for tests that never invoke the returned
// model. Same shape as production, minus side effects.
mock.module('../../src/lib/openrouter-fetch', () => ({
  createOpenRouterCompatibleFetch: () => globalThis.fetch,
}))

const { createLanguageModel } = await import('../../src/agent/provider-factory')

beforeEach(() => {
  lastCallArgs.anthropic = null
  lastCallArgs.openai = null
  lastCallArgs.google = null
  lastCallArgs.openrouter = null
  lastCallArgs.azure = null
})

const CUSTOM_URL = 'https://api.minimax.io/anthropic'

describe('createAnthropicFactory baseUrl handling', () => {
  it('forwards a configured baseUrl as the SDK baseURL option', async () => {
    await createLanguageModel({
      conversationId: 'c1',
      provider: LLM_PROVIDERS.ANTHROPIC,
      model: 'claude-sonnet-4-6',
      apiKey: 'sk-anthropic-test',
      baseUrl: CUSTOM_URL,
    })
    expect(lastCallArgs.anthropic).toMatchObject({
      apiKey: 'sk-anthropic-test',
      baseURL: CUSTOM_URL,
    })
  })

  it('omits baseURL entirely when baseUrl is unset (SDK default preserved)', async () => {
    await createLanguageModel({
      conversationId: 'c1',
      provider: LLM_PROVIDERS.ANTHROPIC,
      model: 'claude-sonnet-4-6',
      apiKey: 'sk-anthropic-test',
    })
    expect(lastCallArgs.anthropic).toEqual({ apiKey: 'sk-anthropic-test' })
    expect(lastCallArgs.anthropic).not.toHaveProperty('baseURL')
  })

  it('omits baseURL when baseUrl is an empty string', async () => {
    // Form serialization can produce empty strings for cleared fields;
    // the short-circuit guard must treat those the same as undefined.
    await createLanguageModel({
      conversationId: 'c1',
      provider: LLM_PROVIDERS.ANTHROPIC,
      model: 'claude-sonnet-4-6',
      apiKey: 'sk-anthropic-test',
      baseUrl: '',
    })
    expect(lastCallArgs.anthropic).not.toHaveProperty('baseURL')
  })
})

describe('createOpenAIFactory baseUrl handling', () => {
  it('forwards a configured baseUrl as the SDK baseURL option', async () => {
    await createLanguageModel({
      conversationId: 'c1',
      provider: LLM_PROVIDERS.OPENAI,
      model: 'gpt-4o',
      apiKey: 'sk-openai-test',
      baseUrl: 'https://gateway.internal/openai',
    })
    expect(lastCallArgs.openai).toMatchObject({
      apiKey: 'sk-openai-test',
      baseURL: 'https://gateway.internal/openai',
    })
  })

  it('omits baseURL when baseUrl is unset (SDK default preserved)', async () => {
    await createLanguageModel({
      conversationId: 'c1',
      provider: LLM_PROVIDERS.OPENAI,
      model: 'gpt-4o',
      apiKey: 'sk-openai-test',
    })
    expect(lastCallArgs.openai).toEqual({ apiKey: 'sk-openai-test' })
    expect(lastCallArgs.openai).not.toHaveProperty('baseURL')
  })
})

describe('createGoogleFactory baseUrl handling', () => {
  it('forwards a configured baseUrl as the SDK baseURL option', async () => {
    await createLanguageModel({
      conversationId: 'c1',
      provider: LLM_PROVIDERS.GOOGLE,
      model: 'gemini-2.5-flash',
      apiKey: 'goog-key',
      baseUrl: 'https://gateway.internal/gemini',
    })
    expect(lastCallArgs.google).toMatchObject({
      apiKey: 'goog-key',
      baseURL: 'https://gateway.internal/gemini',
    })
  })

  it('omits baseURL when baseUrl is unset', async () => {
    await createLanguageModel({
      conversationId: 'c1',
      provider: LLM_PROVIDERS.GOOGLE,
      model: 'gemini-2.5-flash',
      apiKey: 'goog-key',
    })
    expect(lastCallArgs.google).toEqual({ apiKey: 'goog-key' })
  })
})

describe('createOpenRouterFactory baseUrl handling', () => {
  it('forwards a configured baseUrl as the SDK baseURL option', async () => {
    await createLanguageModel({
      conversationId: 'c1',
      provider: LLM_PROVIDERS.OPENROUTER,
      model: 'anthropic/claude-sonnet-4.5',
      apiKey: 'or-key',
      baseUrl: 'https://gateway.internal/openrouter',
    })
    expect(lastCallArgs.openrouter).toMatchObject({
      apiKey: 'or-key',
      baseURL: 'https://gateway.internal/openrouter',
    })
  })

  it('omits baseURL when baseUrl is unset (SDK default endpoint preserved)', async () => {
    await createLanguageModel({
      conversationId: 'c1',
      provider: LLM_PROVIDERS.OPENROUTER,
      model: 'anthropic/claude-sonnet-4.5',
      apiKey: 'or-key',
    })
    expect(lastCallArgs.openrouter).not.toHaveProperty('baseURL')
    // The rest of the OpenRouter-specific args (extraBody, fetch)
    // still flow through unchanged.
    expect(lastCallArgs.openrouter).toMatchObject({
      apiKey: 'or-key',
      extraBody: { reasoning: {} },
    })
  })
})

describe('createAzureFactory baseUrl handling', () => {
  it('forwards a configured baseUrl as the SDK baseURL option alongside resourceName', async () => {
    await createLanguageModel({
      conversationId: 'c1',
      provider: LLM_PROVIDERS.AZURE,
      model: 'gpt-4o',
      apiKey: 'az-key',
      resourceName: 'my-resource',
      baseUrl: 'https://custom-gateway.example.com/openai',
    })
    // Both fields present; the SDK is documented to prefer baseURL
    // when both are set, and our factory forwards whatever the user
    // configured.
    expect(lastCallArgs.azure).toMatchObject({
      apiKey: 'az-key',
      resourceName: 'my-resource',
      baseURL: 'https://custom-gateway.example.com/openai',
    })
  })

  it('accepts baseUrl without resourceName (custom Azure gateway)', async () => {
    await createLanguageModel({
      conversationId: 'c1',
      provider: LLM_PROVIDERS.AZURE,
      model: 'gpt-4o',
      apiKey: 'az-key',
      baseUrl: 'https://custom-gateway.example.com/openai',
    })
    expect(lastCallArgs.azure).toMatchObject({
      apiKey: 'az-key',
      baseURL: 'https://custom-gateway.example.com/openai',
    })
    expect(lastCallArgs.azure).not.toHaveProperty('resourceName')
  })

  it('accepts resourceName without baseUrl (unchanged pre-fix behavior)', async () => {
    await createLanguageModel({
      conversationId: 'c1',
      provider: LLM_PROVIDERS.AZURE,
      model: 'gpt-4o',
      apiKey: 'az-key',
      resourceName: 'my-resource',
    })
    expect(lastCallArgs.azure).toMatchObject({
      apiKey: 'az-key',
      resourceName: 'my-resource',
    })
    expect(lastCallArgs.azure).not.toHaveProperty('baseURL')
  })

  it('throws when neither baseUrl nor resourceName is provided', async () => {
    await expect(
      createLanguageModel({
        conversationId: 'c1',
        provider: LLM_PROVIDERS.AZURE,
        model: 'gpt-4o',
        apiKey: 'az-key',
      }),
    ).rejects.toThrow(/apiKey and either resourceName or baseUrl/)
  })
})
