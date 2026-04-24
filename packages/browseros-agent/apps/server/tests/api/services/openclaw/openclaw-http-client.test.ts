/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it, mock } from 'bun:test'
import { OpenClawSessionNotFoundError } from '../../../../src/api/services/openclaw/errors'
import { OpenClawHttpClient } from '../../../../src/api/services/openclaw/openclaw-http-client'

describe('OpenClawHttpClient', () => {
  const originalFetch = globalThis.fetch

  afterEach(() => {
    globalThis.fetch = originalFetch
  })

  it('maps chat completion deltas into BrowserOS stream events', async () => {
    const fetchMock = mock((_url: string | URL, _init?: RequestInit) =>
      Promise.resolve(
        new Response(
          new ReadableStream({
            start(controller) {
              const encoder = new TextEncoder()
              controller.enqueue(
                encoder.encode(
                  'data: {"choices":[{"delta":{"content":"Hello"}}]}\n\n',
                ),
              )
              controller.enqueue(
                encoder.encode(
                  'data: {"choices":[{"delta":{"content":" world"}}]}\n\n',
                ),
              )
              controller.enqueue(
                encoder.encode(
                  'data: {"choices":[{"delta":{},"finish_reason":"stop"}]}\n\n',
                ),
              )
              controller.enqueue(encoder.encode('data: [DONE]\n\n'))
              controller.close()
            },
          }),
          {
            status: 200,
            headers: { 'Content-Type': 'text/event-stream' },
          },
        ),
      ),
    )
    globalThis.fetch = fetchMock as typeof globalThis.fetch
    const client = new OpenClawHttpClient(18789, async () => 'gateway-token')

    const stream = await client.streamChat({
      agentId: 'research',
      sessionKey: 'session-123',
      message: 'hi',
      history: [{ role: 'assistant', content: 'Earlier reply' }],
    })

    const events = await readEvents(stream)
    const call = fetchMock.mock.calls[0]

    expect(call?.[0]).toBe('http://127.0.0.1:18789/v1/chat/completions')
    expect(call?.[1]).toMatchObject({
      method: 'POST',
      headers: {
        Authorization: 'Bearer gateway-token',
        'Content-Type': 'application/json',
      },
    })
    expect(JSON.parse(String(call?.[1]?.body))).toEqual({
      model: 'openclaw/research',
      stream: true,
      messages: [
        { role: 'assistant', content: 'Earlier reply' },
        { role: 'user', content: 'hi' },
      ],
      user: 'browseros:research:session-123',
    })
    expect(events).toEqual([
      { type: 'text-delta', data: { text: 'Hello' } },
      { type: 'text-delta', data: { text: ' world' } },
      { type: 'done', data: { text: 'Hello world' } },
    ])
  })

  it('uses openclaw for the main agent', async () => {
    const fetchMock = mock(() =>
      Promise.resolve(
        new Response(
          new ReadableStream({
            start(controller) {
              controller.close()
            },
          }),
          {
            status: 200,
            headers: { 'Content-Type': 'text/event-stream' },
          },
        ),
      ),
    )
    globalThis.fetch = fetchMock as typeof globalThis.fetch
    const client = new OpenClawHttpClient(18789, async () => 'gateway-token')

    await client.streamChat({
      agentId: 'main',
      sessionKey: 'session-123',
      message: 'hi',
    })

    const body = JSON.parse(String(fetchMock.mock.calls[0]?.[1]?.body)) as {
      model: string
    }
    expect(body.model).toBe('openclaw')
  })

  it('throws on non-success HTTP responses', async () => {
    globalThis.fetch = mock(() =>
      Promise.resolve(new Response('Unauthorized', { status: 401 })),
    ) as typeof globalThis.fetch
    const client = new OpenClawHttpClient(18789, async () => 'gateway-token')

    await expect(
      client.streamChat({
        agentId: 'research',
        sessionKey: 'session-123',
        message: 'hi',
      }),
    ).rejects.toThrow('Unauthorized')
  })

  it('checks gateway authentication with the current bearer token', async () => {
    const fetchMock = mock(() => Promise.resolve(new Response('{}')))
    globalThis.fetch = fetchMock as typeof globalThis.fetch
    const client = new OpenClawHttpClient(18789, async () => 'gateway-token')

    await expect(client.isAuthenticated()).resolves.toBe(true)

    expect(fetchMock.mock.calls[0]?.[0]).toBe(
      'http://127.0.0.1:18789/v1/models',
    )
    expect(fetchMock.mock.calls[0]?.[1]).toMatchObject({
      method: 'GET',
      headers: {
        Authorization: 'Bearer gateway-token',
      },
    })
  })

  it('treats rejected gateway authentication as unavailable', async () => {
    globalThis.fetch = mock(() =>
      Promise.resolve(new Response('Unauthorized', { status: 401 })),
    ) as typeof globalThis.fetch
    const client = new OpenClawHttpClient(18789, async () => 'gateway-token')

    await expect(client.isAuthenticated()).resolves.toBe(false)
  })

  it('treats failed gateway authentication probes as unavailable', async () => {
    globalThis.fetch = mock(() =>
      Promise.reject(new Error('connect ECONNREFUSED')),
    ) as typeof globalThis.fetch
    const client = new OpenClawHttpClient(18789, async () => 'gateway-token')

    await expect(client.isAuthenticated()).resolves.toBe(false)
  })

  it('surfaces an error when OpenClaw finishes without assistant text', async () => {
    globalThis.fetch = mock(() =>
      Promise.resolve(
        new Response(
          new ReadableStream({
            start(controller) {
              const encoder = new TextEncoder()
              controller.enqueue(
                encoder.encode(
                  'data: {"choices":[{"delta":{},"finish_reason":"stop"}]}\n\n',
                ),
              )
              controller.enqueue(encoder.encode('data: [DONE]\n\n'))
              controller.close()
            },
          }),
          {
            status: 200,
            headers: { 'Content-Type': 'text/event-stream' },
          },
        ),
      ),
    ) as typeof globalThis.fetch
    const client = new OpenClawHttpClient(18789, async () => 'gateway-token')

    const stream = await client.streamChat({
      agentId: 'main',
      sessionKey: 'session-123',
      message: 'hi',
    })

    await expect(readEvents(stream)).resolves.toEqual([
      {
        type: 'error',
        data: {
          message: "Agent couldn't generate a response. Please try again.",
        },
      },
    ])
  })

  it('stops processing batched SSE events after a malformed chunk closes the stream', async () => {
    const fetchMock = mock(() =>
      Promise.resolve(
        new Response(
          new ReadableStream({
            start(controller) {
              const encoder = new TextEncoder()
              controller.enqueue(
                encoder.encode(
                  'data: {"choices":[{"delta":{"content":"Hello"}}]}\n\n' +
                    'data: not-json\n\n' +
                    'data: {"choices":[{"delta":{"content":" world"}}]}\n\n',
                ),
              )
              controller.close()
            },
          }),
          {
            status: 200,
            headers: { 'Content-Type': 'text/event-stream' },
          },
        ),
      ),
    )
    globalThis.fetch = fetchMock as typeof globalThis.fetch
    const client = new OpenClawHttpClient(18789, async () => 'gateway-token')

    const stream = await client.streamChat({
      agentId: 'research',
      sessionKey: 'session-123',
      message: 'hi',
    })

    await expect(readEvents(stream)).resolves.toEqual([
      { type: 'text-delta', data: { text: 'Hello' } },
      {
        type: 'error',
        data: { message: 'Failed to parse OpenClaw chat stream chunk' },
      },
    ])
  })

  describe('getSessionHistory', () => {
    it('sends GET with bearer auth and forwards limit/cursor as query params', async () => {
      const fetchMock = mock(() =>
        Promise.resolve(
          new Response(
            JSON.stringify({
              sessionKey: 'agent:main:main',
              messages: [
                { role: 'user', content: 'hi', messageSeq: 1 },
                { role: 'assistant', content: 'hello', messageSeq: 2 },
              ],
              cursor: null,
              hasMore: false,
            }),
            { status: 200, headers: { 'Content-Type': 'application/json' } },
          ),
        ),
      )
      globalThis.fetch = fetchMock as typeof globalThis.fetch
      const client = new OpenClawHttpClient(18789, async () => 'gateway-token')

      const result = await client.getSessionHistory('agent:main:main', {
        limit: 50,
        cursor: 'abc',
      })

      expect(fetchMock.mock.calls[0]?.[0]).toBe(
        'http://127.0.0.1:18789/sessions/agent%3Amain%3Amain/history?limit=50&cursor=abc',
      )
      expect(fetchMock.mock.calls[0]?.[1]).toMatchObject({
        method: 'GET',
        headers: { Authorization: 'Bearer gateway-token' },
      })
      expect(result).toEqual({
        sessionKey: 'agent:main:main',
        messages: [
          { role: 'user', content: 'hi', messageSeq: 1 },
          { role: 'assistant', content: 'hello', messageSeq: 2 },
        ],
        cursor: null,
        hasMore: false,
      })
    })

    it('omits limit and cursor from the query when undefined', async () => {
      const fetchMock = mock(() =>
        Promise.resolve(
          new Response(JSON.stringify({ sessionKey: 'k', messages: [] }), {
            status: 200,
          }),
        ),
      )
      globalThis.fetch = fetchMock as typeof globalThis.fetch
      const client = new OpenClawHttpClient(18789, async () => 'gateway-token')

      await client.getSessionHistory('k')

      expect(fetchMock.mock.calls[0]?.[0]).toBe(
        'http://127.0.0.1:18789/sessions/k/history',
      )
    })

    it('throws OpenClawSessionNotFoundError on 404', async () => {
      globalThis.fetch = mock(() =>
        Promise.resolve(new Response('not found', { status: 404 })),
      ) as typeof globalThis.fetch
      const client = new OpenClawHttpClient(18789, async () => 'gateway-token')

      await expect(
        client.getSessionHistory('missing-key'),
      ).rejects.toBeInstanceOf(OpenClawSessionNotFoundError)
    })

    it('surfaces the response body on other non-2xx responses', async () => {
      globalThis.fetch = mock(() =>
        Promise.resolve(new Response('boom', { status: 500 })),
      ) as typeof globalThis.fetch
      const client = new OpenClawHttpClient(18789, async () => 'gateway-token')

      await expect(client.getSessionHistory('k')).rejects.toThrow('boom')
    })

    it('propagates the abort signal to fetch', async () => {
      const fetchMock = mock(() =>
        Promise.resolve(
          new Response(JSON.stringify({ sessionKey: 'k', messages: [] }), {
            status: 200,
          }),
        ),
      )
      globalThis.fetch = fetchMock as typeof globalThis.fetch
      const controller = new AbortController()
      const client = new OpenClawHttpClient(18789, async () => 'gateway-token')

      await client.getSessionHistory('k', { signal: controller.signal })

      expect(fetchMock.mock.calls[0]?.[1]?.signal).toBe(controller.signal)
    })
  })

  describe('streamSessionHistory', () => {
    it('parses named history/message SSE events into typed events', async () => {
      const fetchMock = mock(() =>
        Promise.resolve(
          new Response(
            new ReadableStream({
              start(controller) {
                const encoder = new TextEncoder()
                controller.enqueue(
                  encoder.encode(
                    'event: history\ndata: {"sessionKey":"k","messages":[{"role":"user","content":"hi","messageSeq":1}],"cursor":null,"hasMore":false}\n\n',
                  ),
                )
                controller.enqueue(
                  encoder.encode(
                    'event: message\ndata: {"sessionKey":"k","messageSeq":2,"message":{"role":"assistant","content":"hey","messageSeq":2}}\n\n',
                  ),
                )
                controller.close()
              },
            }),
            {
              status: 200,
              headers: { 'Content-Type': 'text/event-stream' },
            },
          ),
        ),
      )
      globalThis.fetch = fetchMock as typeof globalThis.fetch
      const client = new OpenClawHttpClient(18789, async () => 'gateway-token')

      const stream = await client.streamSessionHistory('k', { limit: 20 })

      const events = await readEvents(stream)
      expect(fetchMock.mock.calls[0]?.[0]).toBe(
        'http://127.0.0.1:18789/sessions/k/history?limit=20',
      )
      expect(fetchMock.mock.calls[0]?.[1]).toMatchObject({
        method: 'GET',
        headers: {
          Accept: 'text/event-stream',
          Authorization: 'Bearer gateway-token',
        },
      })
      expect(events).toEqual([
        {
          type: 'history',
          data: {
            sessionKey: 'k',
            messages: [{ role: 'user', content: 'hi', messageSeq: 1 }],
            cursor: null,
            hasMore: false,
          },
        },
        {
          type: 'message',
          data: {
            sessionKey: 'k',
            messageSeq: 2,
            message: { role: 'assistant', content: 'hey', messageSeq: 2 },
          },
        },
      ])
    })

    it('forwards upstream error frames and closes', async () => {
      globalThis.fetch = mock(() =>
        Promise.resolve(
          new Response(
            new ReadableStream({
              start(controller) {
                const encoder = new TextEncoder()
                controller.enqueue(
                  encoder.encode(
                    'event: error\ndata: {"message":"upstream exploded"}\n\n',
                  ),
                )
                controller.close()
              },
            }),
            { status: 200 },
          ),
        ),
      ) as typeof globalThis.fetch
      const client = new OpenClawHttpClient(18789, async () => 'gateway-token')

      const stream = await client.streamSessionHistory('k')

      await expect(readEvents(stream)).resolves.toEqual([
        { type: 'error', data: { message: 'upstream exploded' } },
      ])
    })

    it('throws OpenClawSessionNotFoundError on 404', async () => {
      globalThis.fetch = mock(() =>
        Promise.resolve(new Response('not found', { status: 404 })),
      ) as typeof globalThis.fetch
      const client = new OpenClawHttpClient(18789, async () => 'gateway-token')

      await expect(client.streamSessionHistory('k')).rejects.toBeInstanceOf(
        OpenClawSessionNotFoundError,
      )
    })

    it('closes when the abort signal fires mid-stream', async () => {
      const ac = new AbortController()
      globalThis.fetch = mock(() =>
        Promise.resolve(
          new Response(
            new ReadableStream({
              async start(controller) {
                const encoder = new TextEncoder()
                controller.enqueue(
                  encoder.encode(
                    'event: history\ndata: {"sessionKey":"k","messages":[]}\n\n',
                  ),
                )
                // Keep the stream open; abort should close it from our side.
                await new Promise((resolve) => {
                  ac.signal.addEventListener(
                    'abort',
                    () => resolve(undefined),
                    {
                      once: true,
                    },
                  )
                })
                controller.close()
              },
            }),
            { status: 200 },
          ),
        ),
      ) as typeof globalThis.fetch
      const client = new OpenClawHttpClient(18789, async () => 'gateway-token')

      const stream = await client.streamSessionHistory('k', {
        signal: ac.signal,
      })
      const reader = stream.getReader()
      const first = await reader.read()
      expect(first.done).toBe(false)
      expect(first.value).toMatchObject({ type: 'history' })

      ac.abort()
      const next = await reader.read()
      expect(next.done).toBe(true)
    })
  })
})

async function readEvents(
  stream: ReadableStream<{ type: string; data: Record<string, unknown> }>,
): Promise<Array<{ type: string; data: Record<string, unknown> }>> {
  const reader = stream.getReader()
  const events: Array<{ type: string; data: Record<string, unknown> }> = []

  while (true) {
    const { done, value } = await reader.read()
    if (done) break
    events.push(value)
  }

  return events
}
