import { describe, expect, it } from 'bun:test'
import { createChatRoutes } from '../../../src/api/routes/chat'
import type { ChatProviderLookup } from '../../../src/api/services/chat-provider-config'
import type { ProviderRow } from '../../../src/lib/db/schema'

function storedProvider(): ProviderRow {
  return {
    id: 'anthropic-1',
    profileId: null,
    kind: 'llm',
    type: 'anthropic',
    name: 'My Claude',
    modelId: 'claude-sonnet-4-6',
    reasoningEffort: null,
    isDefault: true,
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
  }
}

const providerStore: ChatProviderLookup = {
  get: async (id) => (id === 'anthropic-1' ? storedProvider() : null),
  getDefault: async () => storedProvider(),
}

function routes() {
  return createChatRoutes({
    browser: { isCdpConnected: () => false } as never,
    browserMcp: {} as never,
    serverPort: 32123,
    providerStore,
  })
}

function chatBody(extra: Record<string, unknown> = {}) {
  return JSON.stringify({
    conversationId: '00000000-0000-4000-8000-000000000003',
    message: 'hello',
    ...extra,
  })
}

/**
 * A browseros chat request is deliberately allowed without the app-origin
 * check, because it normally carries its own credentials. Naming a stored
 * provider changes that: the server supplies the user's key, so any local
 * process could spend it. These cover the line between the two.
 */
describe('chat provider credentials', () => {
  it('refuses an untrusted caller that names a stored provider', async () => {
    const response = await routes().request('/', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: chatBody({
        target: { type: 'browseros', providerId: 'anthropic-1' },
      }),
    })

    expect(response.status).toBe(403)
  })

  // Naming nothing resolves the selected provider, which is the same privilege
  // by a shorter route.
  it('refuses an untrusted caller that relies on the selected provider', async () => {
    const response = await routes().request('/', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: chatBody(),
    })

    expect(response.status).toBe(403)
  })

  // The provider types the server credentials itself. An unknown id means no
  // row is read, so the provenance flag alone would wave these through, and
  // the resolver would then hand over this machine's oauth token or the
  // gateway credential to a caller that proved nothing.
  it.each(['chatgpt-pro', 'github-copilot', 'qwen-code', 'browseros'])(
    'refuses an untrusted caller naming %s with an unknown id',
    async (provider) => {
      const response = await routes().request('/', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: chatBody({
          target: { type: 'browseros', providerId: 'not-stored' },
          provider,
          model: 'some-model',
        }),
      })

      expect(response.status).toBe(403)
    },
  )

  // Bringing your own configuration is what this path always allowed, and it
  // stays allowed: nothing of the user's is being spent.
  it('does not gate a request that brought its own configuration', async () => {
    const response = await routes().request('/', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: chatBody({
        target: { type: 'browseros', providerId: 'not-stored' },
        provider: 'openai',
        model: 'gpt-5.5',
        apiKey: 'sk-caller-own',
      }),
    })

    expect(response.status).not.toBe(403)
  })
})
