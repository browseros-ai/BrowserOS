/** The public client seam is exercised with wire responses, including every error dialect. */
import { expect, test } from 'bun:test'
import { ServerApiClient } from './client'
import { ServerApiError } from './errors'
import { readUiMessageStream } from './stream'

for (const [body, contentType, message, code] of [
  [
    { error: 'Unknown provider' },
    'application/json',
    'Unknown provider',
    undefined,
  ],
  [
    {
      error: {
        name: 'ForbiddenOrigin',
        message: 'Origin not allowed',
        code: 'FORBIDDEN_ORIGIN',
        statusCode: 403,
      },
    },
    'application/json',
    'Origin not allowed',
    'FORBIDDEN_ORIGIN',
  ],
  [
    {
      error: {
        name: 'InternalServerError',
        code: 'unknown',
        title: 'Error',
        message: 'Malformed JSON',
        retryable: false,
        statusCode: 500,
      },
    },
    'application/json',
    'Malformed JSON',
    'unknown',
  ],
  [
    { success: false, error: { name: 'ZodError', message: 'Invalid field' } },
    'application/json',
    'Invalid field',
    undefined,
  ],
  [
    { success: false, message: 'Connection failed', responseTime: 10 },
    'application/json',
    'Connection failed',
    undefined,
  ],
  [
    { message: 'Discovery failed' },
    'application/json',
    'Discovery failed',
    undefined,
  ],
  ['Failed to store token', 'text/plain', 'Failed to store token', undefined],
  [
    { error: { code: -32603, message: 'Internal server error' } },
    'application/json',
    'Internal server error',
    '-32603',
  ],
] as const) {
  test(`normalizes ${message}`, async () => {
    const client = new ServerApiClient('http://localhost:9100/', {
      fetch: async () =>
        new Response(typeof body === 'string' ? body : JSON.stringify(body), {
          status: 400,
          headers: { 'content-type': contentType },
        }),
    })
    try {
      await client.http.GET('/providers')
      throw new Error('Expected a failure')
    } catch (error) {
      expect(error).toBeInstanceOf(ServerApiError)
      expect(error).toMatchObject({ status: 400, message, code })
    }
  })
}

test('typed JSON methods preserve request path, body, credentials and abort signal', async () => {
  const abort = new AbortController()
  // Bun 1.3 reports Request.credentials as "include" even when constructed
  // with omit. Assert the real constructor input rather than that Bun accessor.
  class RecordingRequest extends Request {
    constructor(input: RequestInfo | URL, init?: RequestInit) {
      expect(init?.credentials).toBe('omit')
      super(input, init)
    }
  }
  const client = new ServerApiClient('http://localhost:9100/', {
    Request: RecordingRequest,
    fetch: async (request) => {
      expect(request.url).toBe(
        'http://localhost:9100/providers/contract%2Fprovider',
      )
      expect(request.method).toBe('PUT')
      expect(request.signal.aborted).toBe(false)
      expect(await request.json()).toEqual({
        type: 'openai',
        name: 'Contract',
        modelId: 'fixture',
        contextWindow: 4096,
      })
      return Response.json({ provider: { id: 'contract/provider' } })
    },
  })
  const result = await client.http.PUT('/providers/{providerId}', {
    params: { path: { providerId: 'contract/provider' } },
    body: {
      type: 'openai',
      name: 'Contract',
      modelId: 'fixture',
      contextWindow: 4096,
    },
    signal: abort.signal,
  })
  expect(result.data?.provider.id).toBe('contract/provider')
})

test('chat helpers preserve raw streaming bodies, query ids and 204 absence', async () => {
  const requests: Request[] = []
  const client = new ServerApiClient('http://localhost:9100', {
    fetch: async (request) => {
      requests.push(request)
      return request.url.includes('runId=missing')
        ? new Response(null, { status: 204 })
        : new Response('data: {"type":"start"}\n\ndata: [DONE]\n\n', {
            headers: {
              'content-type': 'text/event-stream',
              'x-vercel-ai-ui-message-stream': 'v1',
            },
          })
    },
  })
  const response = await client.createChat({
    conversationId: '11111111-1111-4111-8111-111111111111',
  })
  expect(response.bodyUsed).toBe(false)
  expect(await Array.fromAsync(readUiMessageStream(response))).toEqual([
    { type: 'start' },
  ])
  const reconnect = await client.reconnectChat('conversation', {
    runId: 'r & 1',
  })
  expect(reconnect?.bodyUsed).toBe(false)
  expect(requests[1]?.url).toBe(
    'http://localhost:9100/chat/conversation/stream?runId=r%20%26%201',
  )
  expect(
    await client.reconnectChat('conversation', { runId: 'missing' }),
  ).toBeNull()
})
