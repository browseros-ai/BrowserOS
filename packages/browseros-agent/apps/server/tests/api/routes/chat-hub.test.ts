import { describe, expect, it } from 'bun:test'
import type { UIMessageChunk } from 'ai'
import { createChatRoutes } from '../../../src/api/routes/chat'
import { ConversationHub } from '../../../src/api/services/conversation-hub'
import { ConversationPresence } from '../../../src/api/services/conversation-presence'

const localServer = {
  server: {
    requestIP: () => ({ address: '127.0.0.1' }),
  },
} as never

describe('/chat server-owned run routes', () => {
  it('serves canonical state and a reconnect stream from the same hub', async () => {
    const hub = new ConversationHub()
    const presence = new ConversationPresence()
    const conversationId = crypto.randomUUID()
    let source!: ReadableStreamDefaultController<UIMessageChunk>
    await hub.start({
      conversationId,
      messages: [{ id: 'user-1', role: 'user', parts: [] }],
      createStream: () =>
        new ReadableStream<UIMessageChunk>({
          start(controller) {
            source = controller
          },
        }),
    })
    const app = route(hub, presence)

    const stateResponse = await app.request(`/${conversationId}/state`)
    expect(stateResponse.status).toBe(200)
    expect(await stateResponse.json()).toMatchObject({
      conversationId,
      status: 'running',
      messages: [{ id: 'user-1' }],
    })

    const streamResponse = await app.request(`/${conversationId}/stream`)
    expect(streamResponse.headers.get('content-type')).toContain(
      'text/event-stream',
    )
    const text = streamResponse.text()
    source.enqueue({ type: 'text-start', id: 'answer-1' })
    source.close()
    expect(await text).toContain('"type":"text-start"')
  })

  it('replays a run that finishes between state hydration and reconnect', async () => {
    const hub = new ConversationHub()
    const conversationId = crypto.randomUUID()
    let source!: ReadableStreamDefaultController<UIMessageChunk>
    await hub.start({
      conversationId,
      messages: [{ id: 'user-1', role: 'user', parts: [] }],
      createStream: () =>
        new ReadableStream<UIMessageChunk>({
          start(controller) {
            source = controller
          },
        }),
    })
    source.enqueue({ type: 'text-start', id: 'answer-1' })
    source.close()
    await eventually(() =>
      expect(hub.getSnapshot(conversationId)?.status).toBe('completed'),
    )

    const response = await route(hub, new ConversationPresence()).request(
      `/${conversationId}/stream`,
    )

    expect(response.status).toBe(200)
    expect(await response.text()).toContain('"type":"text-start"')
  })

  it('exposes presence as an extension-only snapshot-first SSE feed', async () => {
    const hub = new ConversationHub()
    const presence = new ConversationPresence()
    presence.startRun({
      conversationId: crypto.randomUUID(),
      runId: 'run-1',
      tabIds: [42],
    })

    const response = await route(hub, presence).request(
      'http://localhost/presence',
      { headers: { Host: 'localhost' } },
      localServer,
    )
    expect(response.status).toBe(200)
    const reader = response.body?.getReader()
    const first = await reader?.read()
    const text = new TextDecoder().decode(first?.value)
    expect(text).toContain('"type":"snapshot"')
    expect(text).toContain('"tabId":42')
    await reader?.cancel()
  })

  it('cancels a run through the explicit stop endpoint', async () => {
    const hub = new ConversationHub()
    const conversationId = crypto.randomUUID()
    await hub.start({
      conversationId,
      messages: [],
      createStream: () => new ReadableStream<UIMessageChunk>(),
    })

    const response = await route(hub, new ConversationPresence()).request(
      `/${conversationId}/stop`,
      { method: 'POST' },
    )

    expect(await response.json()).toEqual({ stopped: true })
    expect(hub.getSnapshot(conversationId)?.status).toBe('aborted')
  })
})

function route(hub: ConversationHub, presence: ConversationPresence) {
  return createChatRoutes({
    browser: {} as never,
    browserToolRuntime: {} as never,
    serverPort: 9000,
    conversationHub: hub,
    conversationPresence: presence,
  })
}

async function eventually(assertion: () => void): Promise<void> {
  for (let attempt = 0; attempt < 20; attempt += 1) {
    try {
      assertion()
      return
    } catch {
      await Promise.resolve()
    }
  }
  assertion()
}
