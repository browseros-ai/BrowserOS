/** Verify discovery recovery and legacy HTTP result semantics at the app seam. */
import { afterEach, describe, expect, it, mock } from 'bun:test'
import { ServerApiError } from '@browseros/server-api-client'

let discovered: string | Error = new Error('Not ready')
mock.module('@/modules/browseros/agent-server-url.helpers', () => ({
  resolveAgentServerUrl: async () => {
    if (discovered instanceof Error) throw discovered
    return discovered
  },
}))
const { getServerApi, serverApi, serverData, nullOnNotFound, serverResult } =
  await import('./client')
const originalFetch = globalThis.fetch

function testFetch(
  handler: (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>,
): typeof fetch {
  return Object.assign(handler, { preconnect: originalFetch.preconnect })
}

afterEach(() => {
  globalThis.fetch = originalFetch
})

describe('app server client', () => {
  it('recovers discovery and replaces clients when the server port changes', async () => {
    await expect(getServerApi()).rejects.toThrow('Not ready')
    discovered = 'http://localhost:9101'
    const first = await getServerApi()
    expect(await getServerApi()).toBe(first)
    discovered = 'http://localhost:9102'
    expect(await getServerApi()).not.toBe(first)
  })

  it('preserves DELETE JSON and keepalive at the actual Request constructor', async () => {
    const NativeRequest = globalThis.Request
    let keepalive: boolean | undefined
    globalThis.Request = class extends NativeRequest {
      constructor(input: RequestInfo | URL, init?: RequestInit) {
        keepalive = init?.keepalive
        super(input, init)
      }
    }
    const requests: Request[] = []
    globalThis.fetch = testFetch(
      mock(async (input) => {
        if (input instanceof Request) requests.push(input)
        return Response.json({ success: true })
      }),
    )
    try {
      await serverData(
        serverApi('http://localhost:9103').http.DELETE(
          '/klavis/servers/remove',
          { body: { serverName: 'Github' } },
        ),
      )
      expect(requests[0]?.method).toBe('DELETE')
      expect(await requests[0]?.json()).toEqual({ serverName: 'Github' })
      await serverData(
        serverApi('http://localhost:9103').http.DELETE(
          '/chat/{conversationId}',
          { params: { path: { conversationId: 'chat/1' } }, keepalive: true },
        ),
      )
      expect(requests[1]?.url).toBe('http://localhost:9103/chat/chat%2F1')
      expect(keepalive).toBe(true)
    } finally {
      globalThis.Request = NativeRequest
    }
  })

  it('keeps server-side provider failure timing as a result without retrying', async () => {
    let calls = 0
    globalThis.fetch = testFetch(
      mock(async () => {
        calls++
        return Response.json(
          { success: false, message: '[openai] auth failed', responseTime: 42 },
          { status: 400 },
        )
      }),
    )
    const result = await serverResult<{
      success: boolean
      message: string
      responseTime: number
    }>(
      'http://localhost:9104',
      (client) =>
        client.http.POST('/test-provider', {
          body: { provider: 'openai', model: 'fixture' },
        }),
      (
        value,
      ): value is { success: false; message: string; responseTime: number } =>
        typeof value === 'object' &&
        value !== null &&
        'success' in value &&
        value.success === false &&
        'responseTime' in value,
    )
    expect(result).toEqual({
      success: false,
      message: '[openai] auth failed',
      responseTime: 42,
    })
    expect(calls).toBe(1)
  })

  it('normalizes errors once while preserving status wording and tolerant 404 deletion', async () => {
    globalThis.fetch = testFetch(
      mock(async () =>
        Response.json(
          { error: { code: 'auth_failed', message: 'Denied' } },
          { status: 404, statusText: 'Not Found' },
        ),
      ),
    )
    await expect(
      serverData(
        serverApi('http://localhost:9104').http.GET('/providers'),
        (error, response) => `Failed (${error.status} ${response?.statusText})`,
      ),
    ).rejects.toMatchObject({
      name: 'ServerApiError',
      status: 404,
      code: 'auth_failed',
      message: 'Failed (404 Not Found)',
    })
    expect(
      await nullOnNotFound(
        serverApi('http://localhost:9104').http.DELETE(
          '/providers/{providerId}',
          { params: { path: { providerId: 'missing' } } },
        ),
      ),
    ).toBeNull()
    globalThis.fetch = testFetch(
      mock(async () => Response.json({ error: 'Busy' }, { status: 409 })),
    )
    await expect(
      nullOnNotFound(
        serverApi('http://localhost:9104').http.DELETE(
          '/providers/{providerId}',
          { params: { path: { providerId: 'busy' } } },
        ),
      ),
    ).rejects.toBeInstanceOf(ServerApiError)
  })
})
