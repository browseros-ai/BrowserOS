/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it, mock } from 'bun:test'
import { OpenClawHttpChatClient } from '../../../../src/api/services/openclaw/openclaw-http-chat-client'

describe('OpenClawHttpChatClient', () => {
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
    const client = new OpenClawHttpChatClient(
      18789,
      async () => 'gateway-token',
    )

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
    const client = new OpenClawHttpChatClient(
      18789,
      async () => 'gateway-token',
    )

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
    const client = new OpenClawHttpChatClient(
      18789,
      async () => 'gateway-token',
    )

    await expect(
      client.streamChat({
        agentId: 'research',
        sessionKey: 'session-123',
        message: 'hi',
      }),
    ).rejects.toThrow('Unauthorized')
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
    const client = new OpenClawHttpChatClient(
      18789,
      async () => 'gateway-token',
    )

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
    const client = new OpenClawHttpChatClient(
      18789,
      async () => 'gateway-token',
    )

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
