import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import { buildChatRequestBody } from '../messaging/server/buildChatRequestBody'

const storageValues = new Map<string, unknown>()
const fetchBodies: Array<Record<string, unknown>> = []
const originalFetch = globalThis.fetch

const createBrowserOSProvider = () => ({
  id: 'browseros',
  type: 'browseros',
  name: 'BrowserOS',
  modelId: 'browseros-auto',
  supportsImages: true,
  contextWindow: 200000,
  temperature: 0.2,
  createdAt: 0,
  updatedAt: 0,
})

// Total replacements are intentional here: these storage/helper
// modules pull in wxt/storage + generated graphql code that requires
// build-time output not present in this test context. No other test
// imports from these modules, so cross-file pollution isn't a risk.
// Per-file worker isolation (Level 3 in the 2026-07-17 test
// reliability audit) covers the general class regardless.
mock.module('@/lib/llm-providers/storage', () => ({
  DEFAULT_PROVIDER_ID: 'browseros',
  createDefaultBrowserOSProvider: createBrowserOSProvider,
  createDefaultProvidersConfig: () => [createBrowserOSProvider()],
  loadProviders: async () =>
    (storageValues.get('providers') as LlmProviderConfig[]) ?? [],
  providersStorage: {
    getValue: async () => storageValues.get('providers'),
    setValue: async (value: LlmProviderConfig[]) => {
      storageValues.set('providers', value)
    },
    watch: () => () => {},
  },
  defaultProviderIdStorage: {
    getValue: async () => storageValues.get('defaultProviderId'),
    setValue: async (value: string) => {
      storageValues.set('defaultProviderId', value)
    },
    watch: () => () => {},
  },
}))

// The provider list is a request now, not a storage read. Mocked here so the
// fetch stub below still sees only the chat call it is asserting on.
mock.module('@/modules/llm-providers/llm-providers.api', () => ({
  listProvidersOrNull: async () =>
    storageValues.has('unreachable')
      ? null
      : ((storageValues.get('providers') as LlmProviderConfig[]) ?? []),
}))

mock.module('@/lib/browseros/helpers', () => ({
  getAgentServerUrl: async () => 'http://127.0.0.1:9105',
  getMcpServerUrl: async () => 'http://127.0.0.1:9106/mcp',
  getHealthCheckUrl: async () => 'http://127.0.0.1:9106/system/health',
  getProxyPort: async () => 9106,
}))

mock.module('@/lib/mcp/mcpServerStorage', () => ({
  mcpServerStorage: {
    getValue: async () => [],
  },
}))

mock.module('@/lib/messaging/server/buildChatRequestBody', () => ({
  buildChatRequestBody,
}))

mock.module('../personalization/personalizationStorage', () => ({
  personalizationStorage: {
    getValue: async () => 'Use concise output.',
  },
}))

beforeEach(() => {
  storageValues.clear()
  fetchBodies.length = 0
  storageValues.set('providers', providers)
  storageValues.set('defaultProviderId', 'anthropic-sonnet')
  globalThis.fetch = mock(async (_url, init) => {
    fetchBodies.push(JSON.parse(String(init?.body ?? '{}')))
    return new Response(
      [
        'data: {"type":"text-delta","id":"message","delta":"done"}',
        '',
        'data: {"type":"finish","finishReason":"stop"}',
        '',
        '',
      ].join('\n'),
      {
        status: 200,
        headers: { 'Content-Type': 'text/event-stream' },
      },
    )
  }) as unknown as typeof fetch
})

afterEach(() => {
  globalThis.fetch = originalFetch
})

describe('scheduled provider resolution', () => {
  it('uses an explicit scheduled provider', async () => {
    const { getChatServerResponse } = await import('./getChatServerResponse')

    await getChatServerResponse({
      message: 'Run my schedule',
      providerId: 'anthropic-sonnet',
    })

    expect(fetchBodies[0]).toMatchObject({
      provider: 'anthropic',
      providerName: 'Anthropic Sonnet',
      model: 'claude-sonnet-4-6',
    })
  })

  it('uses an explicit refine provider', async () => {
    globalThis.fetch = mock(async (_url, init) => {
      fetchBodies.push(JSON.parse(String(init?.body ?? '{}')))
      return Response.json({ success: true, refined: 'Refined prompt' })
    }) as unknown as typeof fetch

    const { refinePrompt } = await import('./refine-prompt')

    await refinePrompt({
      prompt: 'Check mail',
      name: 'Morning brief',
      providerId: 'anthropic-sonnet',
    })

    expect(fetchBodies[0]).toMatchObject({
      provider: 'anthropic',
      model: 'claude-sonnet-4-6',
    })
  })
})

const timestamp = 1000

const providers: LlmProviderConfig[] = [
  {
    id: 'browseros',
    type: 'browseros',
    name: 'BrowserOS',
    modelId: 'browseros-auto',
    supportsImages: true,
    contextWindow: 200000,
    temperature: 0.2,
    createdAt: timestamp,
    updatedAt: timestamp,
  },
  {
    id: 'anthropic-sonnet',
    type: 'anthropic',
    name: 'Anthropic Sonnet',
    modelId: 'claude-sonnet-4-6',
    apiKey: 'sk-ant',
    supportsImages: true,
    contextWindow: 200000,
    temperature: 0.2,
    createdAt: timestamp,
    updatedAt: timestamp,
  },
]

describe('provider resolution when the server is unreachable', () => {
  // The list being unreachable says nothing about whether the chosen provider
  // exists, so running anyway would spend the wrong credentials on the wrong
  // model. The job runner turns this into a failed run the user can see.
  it('fails a scheduled job that named a provider', async () => {
    storageValues.set('unreachable', true)
    const { getChatServerResponse } = await import('./getChatServerResponse')

    await expect(
      getChatServerResponse({
        message: 'Run my schedule',
        providerId: 'anthropic-sonnet',
      }),
    ).rejects.toThrow('Cannot reach the BrowserOS server')

    expect(fetchBodies).toHaveLength(0)
  })

  it('fails a refine that named a provider', async () => {
    storageValues.set('unreachable', true)
    const { refinePrompt } = await import('./refine-prompt')

    await expect(
      refinePrompt({
        prompt: 'Check mail',
        name: 'Morning brief',
        providerId: 'anthropic-sonnet',
      }),
    ).rejects.toThrow('Cannot reach the BrowserOS server')
  })

  // A job that named nothing still has a choice behind it: the configured
  // default. Its id is in extension storage but its model and credentials are
  // in the list that failed to load, so the built-in is not a safe stand-in.
  it('fails a scheduled job that relies on the configured default', async () => {
    storageValues.set('unreachable', true)
    const { getChatServerResponse } = await import('./getChatServerResponse')

    await expect(
      getChatServerResponse({ message: 'Run my schedule' }),
    ).rejects.toThrow('Cannot reach the BrowserOS server')

    expect(fetchBodies).toHaveLength(0)
  })

  // An empty list is a different answer from an unreachable one: the server
  // replied and really has no providers, so the built-in is correct.
  it('still falls back to the built-in provider when the server has none', async () => {
    storageValues.set('providers', [])
    const { getChatServerResponse } = await import('./getChatServerResponse')

    await getChatServerResponse({ message: 'Run my schedule' })

    expect(fetchBodies[0]).toMatchObject({ provider: 'browseros' })
  })

  // A provider that was genuinely deleted still falls back, as before. Only
  // the unreachable case is treated as unsafe.
  it('falls back when the named provider no longer exists', async () => {
    const { getChatServerResponse } = await import('./getChatServerResponse')

    await getChatServerResponse({
      message: 'Run my schedule',
      providerId: 'deleted-provider',
    })

    expect(fetchBodies[0]).toMatchObject({ provider: 'anthropic' })
  })
})
