import { describe, expect, it } from 'bun:test'
import {
  type ChatProviderLookup,
  hydrateChatProvider,
} from '../../../src/api/services/chat-provider-config'
import type { BrowserOsChatRequest } from '../../../src/api/types'
import type { ProviderRow } from '../../../src/lib/db/schema'

function row(overrides: Partial<ProviderRow> = {}): ProviderRow {
  return {
    id: 'anthropic-1',
    profileId: null,
    kind: 'llm',
    type: 'anthropic',
    name: 'My Claude',
    modelId: 'claude-sonnet-4-6',
    reasoningEffort: null,
    isDefault: false,
    createdAt: 1,
    updatedAt: 2,
    baseUrl: null,
    supportsImages: true,
    contextWindow: 200000,
    temperature: 0.2,
    apiKey: 'sk-stored',
    accessKeyId: null,
    secretAccessKey: null,
    sessionToken: null,
    resourceName: null,
    region: null,
    reasoningSummary: null,
    workingDirectory: null,
    customConfig: null,
    ...overrides,
  }
}

function lookup(rows: ProviderRow[] = []): ChatProviderLookup {
  return {
    get: async (id) => rows.find((r) => r.id === id) ?? null,
    getDefault: async () => rows.find((r) => r.isDefault) ?? null,
  }
}

function request(
  overrides: Record<string, unknown> = {},
): BrowserOsChatRequest {
  return {
    conversationId: '00000000-0000-4000-8000-000000000001',
    message: 'hello',
    target: { type: 'browseros', providerId: undefined },
    ...overrides,
  } as BrowserOsChatRequest
}

describe('hydrateChatProvider', () => {
  it('fills the configuration from a named provider', async () => {
    const result = await hydrateChatProvider(
      request({ target: { type: 'browseros', providerId: 'anthropic-1' } }),
      lookup([row()]),
    )

    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.request).toMatchObject({
      provider: 'anthropic',
      model: 'claude-sonnet-4-6',
      apiKey: 'sk-stored',
      contextWindowSize: 200000,
    })
  })

  // The point of the change: a body carrying nothing but a message and a
  // conversation still resolves, because the server knows what is selected.
  it('falls back to the selected provider when none is named', async () => {
    const result = await hydrateChatProvider(
      request(),
      lookup([row({ isDefault: true })]),
    )

    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.request.target.providerId).toBe('anthropic-1')
    expect(result.request.provider).toBe('anthropic')
  })

  // The row is the source of truth, so a client holding a copy from before an
  // edit does not get to override it.
  it('prefers the stored row over anything sent inline', async () => {
    const result = await hydrateChatProvider(
      request({
        target: { type: 'browseros', providerId: 'anthropic-1' },
        provider: 'openai',
        model: 'stale-model',
        apiKey: 'sk-stale',
      }),
      lookup([row()]),
    )

    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.request).toMatchObject({
      provider: 'anthropic',
      model: 'claude-sonnet-4-6',
      apiKey: 'sk-stored',
    })
  })

  // A client from before this change ships the whole configuration and never
  // relies on the lookup, so it has to keep working against a server that
  // knows nothing about the id it names.
  it('keeps an inline configuration when the server has no such row', async () => {
    const result = await hydrateChatProvider(
      request({
        target: { type: 'browseros', providerId: 'unknown-1' },
        provider: 'openai',
        model: 'gpt-5.5',
        apiKey: 'sk-inline',
      }),
      lookup([]),
    )

    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.request).toMatchObject({
      provider: 'openai',
      model: 'gpt-5.5',
      apiKey: 'sk-inline',
    })
  })

  it('refuses a request that names nothing and has no selection', async () => {
    const result = await hydrateChatProvider(request(), lookup([]))

    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.error).toMatch(/none is selected/)
  })

  // Now that either kind can be the default, the browseros path has to say so
  // rather than running the conversation on some other provider.
  it('refuses to serve a coding agent on the browseros path', async () => {
    const result = await hydrateChatProvider(
      request(),
      lookup([
        row({ id: 'acp-1', kind: 'acp', type: 'claude', isDefault: true }),
      ]),
    )

    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.error).toMatch(/coding agent/)
  })

  // The route gates on this: supplying the user's credentials is a privilege
  // the request does not carry on its own.
  it('reports when the configuration came from storage', async () => {
    const result = await hydrateChatProvider(
      request({ target: { type: 'browseros', providerId: 'anthropic-1' } }),
      lookup([row()]),
    )

    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.usedStoredProvider).toBe(true)
  })

  it('reports when the request brought its own configuration', async () => {
    const result = await hydrateChatProvider(
      request({
        target: { type: 'browseros', providerId: 'unknown-1' },
        provider: 'openai',
        model: 'gpt-5.5',
        apiKey: 'sk-inline',
      }),
      lookup([]),
    )

    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.usedStoredProvider).toBe(false)
  })
})
