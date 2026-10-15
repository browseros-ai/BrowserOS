/** Exercise the installed SDK reducer and detachment through the shared reader. */
import { describe, expect, it } from 'bun:test'
import { readUIMessageStream, type UIMessageChunk } from 'ai'
import { ServerChatTransport } from './chat-transport'

class InspectableTransport extends ServerChatTransport {
  decode(body: ReadableStream<Uint8Array>) {
    return this.processResponseStream(body)
  }
}

function stream(events: unknown[], oneByte = false, done = true) {
  const wire =
    events.map((event) => `data: ${JSON.stringify(event)}\r\n\r\n`).join('') +
    (done ? 'data: [DONE]\r\n\r\n' : '')
  const bytes = new TextEncoder().encode(wire)
  return new ReadableStream<Uint8Array>({
    start(controller) {
      if (oneByte)
        for (const byte of bytes) controller.enqueue(Uint8Array.of(byte))
      else controller.enqueue(bytes)
      controller.close()
    },
  })
}

describe('server chat transport', () => {
  it('closes normally when a stop race ends the body without DONE', async () => {
    const events: UIMessageChunk[] = [
      { type: 'start' },
      { type: 'abort', reason: 'Stopped' },
    ]
    expect(
      await Array.fromAsync(
        new InspectableTransport().decode(stream(events, true, false)),
      ),
    ).toEqual(events)
  })

  it('still rejects an invalid chunk when DONE is optional', async () => {
    await expect(
      Array.fromAsync(
        new InspectableTransport().decode(
          stream([{ type: 'text-delta', id: 't' }], false, false),
        ),
      ),
    ).rejects.toMatchObject({ name: 'ServerApiError', code: 'invalid_stream' })
  })

  it('feeds text, dynamic tools and metadata into the installed AI SDK reducer', async () => {
    const output = new InspectableTransport().decode(
      stream(
        [
          { type: 'start', messageId: 'assistant-1' },
          { type: 'start-step' },
          { type: 'text-start', id: 't' },
          { type: 'text-delta', id: 't', delta: 'Hello 🌍' },
          {
            type: 'text-end',
            id: 't',
            providerMetadata: { fixture: { source: 'server' } },
          },
          {
            type: 'tool-input-available',
            toolCallId: 'call-1',
            toolName: 'click',
            input: { id: 10 },
            dynamic: true,
          },
          {
            type: 'tool-output-available',
            toolCallId: 'call-1',
            output: { content: [{ type: 'text', text: 'clicked' }] },
            dynamic: true,
          },
          { type: 'finish-step' },
          { type: 'finish', finishReason: 'stop' },
        ],
        true,
      ),
    )
    const messages = await Array.fromAsync(
      readUIMessageStream({ stream: output }),
    )
    expect(messages.at(-1)).toMatchObject({
      id: 'assistant-1',
      role: 'assistant',
      parts: [
        { type: 'step-start' },
        {
          type: 'text',
          text: 'Hello 🌍',
          state: 'done',
          providerMetadata: { fixture: { source: 'server' } },
        },
        {
          type: 'dynamic-tool',
          toolCallId: 'call-1',
          toolName: 'click',
          state: 'output-available',
          input: { id: 10 },
          output: { content: [{ type: 'text', text: 'clicked' }] },
        },
      ],
    })
  })

  it('cancels a pending read and releases the body without sending stop', async () => {
    let cancelled = 0
    const body = new ReadableStream<Uint8Array>({
      cancel() {
        cancelled++
      },
    })
    const reader = new InspectableTransport().decode(body).getReader()
    const pending = reader.read()
    await Promise.resolve()
    await reader.cancel('panel closed')
    expect(await pending).toEqual({ done: true, value: undefined })
    expect(cancelled).toBe(1)
    expect(body.locked).toBe(false)
  })

  it('releases the source when detached before the first pull', async () => {
    let cancelled = 0
    const body = new ReadableStream<Uint8Array>({
      cancel() {
        cancelled++
      },
    })
    const output = new InspectableTransport().decode(body)
    await output.cancel('closed before reading')
    expect(cancelled).toBe(1)
    expect(body.locked).toBe(false)
  })

  it('retains 204 absence, stale-run 409 and a single reconnect GET', async () => {
    const methods: string[] = []
    let status = 204
    const transport = new ServerChatTransport({
      api: 'http://localhost:9100/chat',
      fetch: Object.assign(
        async (_input: RequestInfo | URL, init?: RequestInit) => {
          methods.push(init?.method ?? '')
          return status === 204
            ? new Response(null, { status })
            : Response.json({ error: 'Run changed' }, { status })
        },
        { preconnect: fetch.preconnect },
      ),
    })
    expect(
      await transport.reconnectToStream({ chatId: 'conversation' }),
    ).toBeNull()
    status = 409
    await expect(
      transport.reconnectToStream({ chatId: 'conversation' }),
    ).rejects.toThrow()
    expect(methods).toEqual(['GET', 'GET'])
  })

  it('keeps the prepared POST body and does not retry chat failures', async () => {
    const bodies: unknown[] = []
    const transport = new ServerChatTransport({
      prepareSendMessagesRequest: () => ({
        api: 'http://localhost:9100/chat',
        body: {
          conversationId: 'chat-1',
          target: { type: 'browseros' },
          message: 'hello',
        },
      }),
      fetch: Object.assign(
        async (_input: RequestInfo | URL, init?: RequestInit) => {
          bodies.push(JSON.parse(String(init?.body)))
          return Response.json(
            { error: { message: 'Provider unavailable' } },
            { status: 503 },
          )
        },
        { preconnect: fetch.preconnect },
      ),
    })
    await expect(
      transport.sendMessages({
        chatId: 'chat-1',
        trigger: 'submit-message',
        messageId: undefined,
        messages: [],
        abortSignal: undefined,
      }),
    ).rejects.toThrow()
    expect(bodies).toEqual([
      {
        conversationId: 'chat-1',
        target: { type: 'browseros' },
        message: 'hello',
      },
    ])
  })
})
