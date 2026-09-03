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

// Only the refine path still resolves a provider on this side; the scheduled
// path names an id and lets the server do it.
const providers: LlmProviderConfig[] = [
  {
    id: 'anthropic-sonnet',
    type: 'anthropic',
    name: 'Anthropic Sonnet',
    modelId: 'claude-sonnet-4-6',
    supportsImages: true,
    contextWindow: 200000,
    temperature: 0.2,
    createdAt: 0,
    updatedAt: 0,
  },
]

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
  // The runner names the provider and stops there. Its model, endpoint and
  // credentials are resolved from the id on the server, which is what let the
  // client-side lookup and its unreachable-versus-empty guard go: the guard
  // existed because a failed lookup and an empty list looked the same, and a
  // job risked running on the built-in provider with the wrong credentials.
  it('names the provider and sends no configuration', async () => {
    const { getChatServerResponse } = await import('./getChatServerResponse')

    await getChatServerResponse({
      message: 'Run my schedule',
      providerId: 'anthropic-sonnet',
    })

    expect(fetchBodies[0]).toMatchObject({
      target: { type: 'browseros', providerId: 'anthropic-sonnet' },
      isScheduledTask: true,
    })
    for (const field of ['apiKey', 'model', 'provider', 'baseUrl']) {
      expect(field in fetchBodies[0]).toBe(false)
    }
  })

  // A job created without picking a provider names none, and the server uses
  // whichever is selected.
  it('leaves the provider unnamed when the job has none', async () => {
    const { getChatServerResponse } = await import('./getChatServerResponse')

    await getChatServerResponse({ message: 'Run my schedule' })

    expect(
      (fetchBodies[0] as { target: { providerId?: string } }).target.providerId,
    ).toBeUndefined()
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
