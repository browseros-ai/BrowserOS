import { describe, expect, it } from 'bun:test'
import { Chat } from '@ai-sdk/react'
import { DefaultChatTransport, type UIMessage, type UIMessageChunk } from 'ai'
import { createChatRoutes } from '../../../server/src/api/routes/chat'
import { ConversationRuns } from '../../../server/src/api/services/conversation-runs'
import { attachConversationRun } from './conversation-run-attachment'
import { conversationReconnectUrl } from './conversation-run-client'

describe('panel run attachment through HTTP and the real SDK reducer', () => {
  it('multicasts a partial response and replays without duplicate text after a reconnect error', async () => {
    const f = await fixture()
    let failed = false
    const transientFetch = (async (
      input: RequestInfo | URL,
      init?: RequestInit,
    ) => {
      if (String(input).includes('/stream') && !failed) {
        failed = true
        return new Response(null, { status: 503 })
      }
      return f.fetch(input, init)
    }) as typeof fetch
    const first = f.chat()
    const second = f.chat(transientFetch)
    const firstAbort = new AbortController()
    const secondAbort = new AbortController()
    const attachingFirst = f.attach(first, firstAbort.signal)
    f.source.enqueue({ type: 'start', messageId: 'reply' })
    f.source.enqueue({ type: 'text-start', id: 'text' })
    f.source.enqueue({ type: 'text-delta', id: 'text', delta: 'hello ' })
    await eventually(() => expect(answer(first)).toBe('hello '))
    const attachingSecond = f.attach(second, secondAbort.signal, transientFetch)
    await eventually(() => expect(answer(second)).toBe('hello '))
    f.source.enqueue({ type: 'text-delta', id: 'text', delta: 'world' })
    f.source.enqueue({ type: 'text-end', id: 'text' })
    f.source.enqueue({ type: 'finish', finishReason: 'stop' })
    f.source.close()
    await Promise.all([attachingFirst, attachingSecond])
    expect(first.messages).toEqual(second.messages)
    expect(first.messages).toHaveLength(2)
    expect(answer(first)).toBe('hello world')
    expect(failed).toBe(true)
  })

  it('ignores delayed hydration after its view is replaced without stopping the shared run', async () => {
    const f = await fixture()
    let release!: () => void
    let requested!: () => void
    const requestStarted = new Promise<void>((resolve) => {
      requested = resolve
    })
    const blocked = new Promise<void>((resolve) => {
      release = resolve
    })
    const delayedFetch = (async (
      input: RequestInfo | URL,
      init?: RequestInit,
    ) => {
      const response = await f.fetch(input, init)
      requested()
      await blocked // Deliberately ignores cancellation, as a late callback can.
      return response
    }) as typeof fetch
    const oldChat = f.chat()
    const controller = new AbortController()
    const attaching = f.attach(oldChat, controller.signal, delayedFetch)
    await requestStarted
    controller.abort()
    release()
    await attaching
    expect(oldChat.messages).toEqual([])
    expect(f.runs.getSnapshot(f.conversationId)?.status).toBe('running')
    await f.runs.stop(f.conversationId)
  })

  it('reconstructs a completed turn from the prepared seed instead of its final assistant message', async () => {
    const f = await fixture()
    f.source.enqueue({ type: 'start', messageId: 'reply' })
    f.source.enqueue({ type: 'text-start', id: 'text' })
    f.source.enqueue({ type: 'text-delta', id: 'text', delta: 'finished' })
    f.source.enqueue({ type: 'text-end', id: 'text' })
    f.source.enqueue({ type: 'finish', finishReason: 'stop' })
    f.runs.updateMessages(f.conversationId, f.runId, [
      { id: 'user', role: 'user', parts: [{ type: 'text', text: 'question' }] },
      {
        id: 'reply',
        role: 'assistant',
        parts: [{ type: 'text', text: 'finished' }],
      },
    ])
    f.source.close()
    await eventually(() =>
      expect(f.runs.getSnapshot(f.conversationId)?.status).toBe('completed'),
    )
    const chat = f.chat()
    await f.attach(chat, new AbortController().signal)
    expect(answer(chat)).toBe('finished')
    expect(chat.messages).toHaveLength(2)
  })
})

async function fixture() {
  const runs = new ConversationRuns()
  const conversationId = crypto.randomUUID()
  let source!: ReadableStreamDefaultController<UIMessageChunk>
  const { runId } = await runs.start({
    conversationId,
    messages: [
      { id: 'user', role: 'user', parts: [{ type: 'text', text: 'question' }] },
    ],
    createStream: () =>
      new ReadableStream<UIMessageChunk>({
        start(controller) {
          source = controller
        },
      }),
  })
  const routes = createChatRoutes({
    browser: {} as never,
    browserMcp: {} as never,
    serverPort: 9000,
    conversationRuns: runs,
  })
  const fetchImpl = ((input: RequestInfo | URL, init?: RequestInit) =>
    routes.request(String(input).replace('/chat/', '/'), init)) as typeof fetch
  return {
    runs,
    conversationId,
    runId,
    source,
    fetch: fetchImpl,
    chat: (transportFetch = fetchImpl) =>
      new Chat<UIMessage>({
        transport: new DefaultChatTransport({
          fetch: transportFetch,
          prepareReconnectToStreamRequest: ({ body }) => ({
            api: conversationReconnectUrl(
              'http://localhost',
              conversationId,
              String(body?.runId),
            ),
          }),
        }),
      }),
    attach: (
      chat: Chat<UIMessage>,
      signal: AbortSignal,
      stateFetch = fetchImpl,
    ) =>
      attachConversationRun({
        chat,
        signal,
        conversationId,
        runId,
        serverUrl: 'http://localhost',
        fetch: stateFetch,
        retryMs: 1,
      }),
  }
}

function answer(chat: Chat<UIMessage>): string {
  return chat.messages
    .filter((m) => m.role === 'assistant')
    .flatMap((m) => m.parts)
    .filter((part) => part.type === 'text')
    .map((part) => part.text)
    .join('')
}

async function eventually(assertion: () => void) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    try {
      assertion()
      return
    } catch {
      await Bun.sleep(2)
    }
  }
  assertion()
}
