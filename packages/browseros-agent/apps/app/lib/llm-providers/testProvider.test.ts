import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { testProvider } from './testProvider'
import type { LlmProviderConfig } from './types'

let lastCall: { url: string; body: Record<string, unknown> } | null = null
let originalFetch: typeof globalThis.fetch

beforeEach(() => {
  lastCall = null
  originalFetch = globalThis.fetch
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    lastCall = {
      url: typeof input === 'string' ? input : input.toString(),
      body: init?.body ? JSON.parse(init.body as string) : {},
    }
    return {
      ok: true,
      json: async () => ({ success: true, message: 'ok' }),
    } as Response
  }) as unknown as typeof globalThis.fetch
})

afterEach(() => {
  globalThis.fetch = originalFetch
})

function baseProvider(
  overrides: Partial<LlmProviderConfig> = {},
): LlmProviderConfig {
  return {
    id: 'p-1',
    type: 'anthropic',
    name: 'Anthropic',
    modelId: 'claude-sonnet-4-6',
    supportsImages: true,
    contextWindow: 200000,
    temperature: 0.2,
    createdAt: 1,
    updatedAt: 1,
    apiKey: 'sk-test',
    ...overrides,
  }
}

describe('testProvider — request body', () => {
  it('forwards model-backed fields for non-ACP providers', async () => {
    await testProvider(baseProvider(), 'http://127.0.0.1:9000')
    expect(lastCall?.url).toBe('http://127.0.0.1:9000/test-provider')
    expect(lastCall?.body).toMatchObject({
      provider: 'anthropic',
      model: 'claude-sonnet-4-6',
      apiKey: 'sk-test',
    })
  })

  it('forwards ACP fields when present', async () => {
    await testProvider(
      baseProvider({
        type: 'claude-code',
        apiKey: undefined,
        acpAgentId: 'claude',
        acpFixedWorkspacePath: '/tmp/x',
      }),
      'http://127.0.0.1:9000',
    )
    expect(lastCall?.body).toMatchObject({
      provider: 'claude-code',
      acpAgentId: 'claude',
      acpFixedWorkspacePath: '/tmp/x',
    })
  })

  it('forwards acp-custom command for the probe spawn path', async () => {
    await testProvider(
      baseProvider({
        type: 'acp-custom',
        acpAgentId: 'my-agent',
        acpCommand: 'my-bin acp',
      }),
      'http://127.0.0.1:9000',
    )
    expect(lastCall?.body).toMatchObject({
      provider: 'acp-custom',
      acpAgentId: 'my-agent',
      acpCommand: 'my-bin acp',
    })
  })
})

describe('testProvider — client-side fetch failure (issue #1844)', () => {
  it('wraps a network fetch error as "could not reach BrowserOS server"', async () => {
    globalThis.fetch = (async () => {
      throw new TypeError('Failed to fetch')
    }) as unknown as typeof globalThis.fetch

    const result = await testProvider(baseProvider(), 'http://127.0.0.1:9200')
    expect(result.success).toBe(false)
    // Message must call out the LOCAL server and the URL we tried,
    // not blame the user's provider config. Guards against a future
    // refactor re-introducing the bare `error.message` return which
    // reads as if the port the user typed was dropped.
    expect(result.message).toContain('local BrowserOS server')
    expect(result.message).toContain('http://127.0.0.1:9200')
    expect(result.message).toContain('Failed to fetch')
    expect(result.responseTime).toBeGreaterThanOrEqual(0)
  })

  it('wraps a JSON-parse failure the same way (server returned non-JSON)', async () => {
    globalThis.fetch = (async () =>
      ({
        ok: true,
        json: async () => {
          throw new SyntaxError('Unexpected token < in JSON')
        },
      }) as unknown as Response) as unknown as typeof globalThis.fetch

    const result = await testProvider(baseProvider(), 'http://127.0.0.1:9200')
    expect(result.success).toBe(false)
    expect(result.message).toContain('local BrowserOS server')
    expect(result.message).toContain('http://127.0.0.1:9200')
    expect(result.message).toContain('Unexpected token')
  })

  it('does NOT wrap a server-returned failure (provider error passes through)', async () => {
    // Server-side probe returns success: false with a `[<provider>]`
    // prefixed message. That is the happy path and must land in the
    // return statement unmodified so the user sees the real provider
    // error (bad URL, 401, timeout, etc.).
    globalThis.fetch = (async () =>
      ({
        ok: true,
        json: async () => ({
          success: false,
          message: '[anthropic] 401 Unauthorized',
        }),
      }) as unknown as Response) as unknown as typeof globalThis.fetch

    const result = await testProvider(baseProvider(), 'http://127.0.0.1:9200')
    expect(result.success).toBe(false)
    expect(result.message).toBe('[anthropic] 401 Unauthorized')
    expect(result.message).not.toContain('local BrowserOS server')
  })
})
