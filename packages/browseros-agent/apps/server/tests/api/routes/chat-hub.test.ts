import { describe, expect, it } from 'bun:test'
import type { UIMessageChunk } from 'ai'
import { createChatRoutes } from '../../../src/api/routes/chat'
import { ConversationRuns } from '../../../src/api/services/conversation-runs'

const localServer = {
  server: {
    requestIP: () => ({ address: '127.0.0.1' }),
  },
} as never

describe('/chat server-owned run routes', () => {
  it('serves canonical state and a reconnect stream from the same hub', async () => {
    const runs = new ConversationRuns()
    const conversationId = crypto.randomUUID()
    let source!: ReadableStreamDefaultController<UIMessageChunk>
    await runs.start({
      conversationId,
      messages: [{ id: 'user-1', role: 'user', parts: [] }],
      createStream: () =>
        new ReadableStream<UIMessageChunk>({
          start(controller) {
            source = controller
          },
        }),
    })
    const app = route(runs)

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
    const runs = new ConversationRuns()
    const conversationId = crypto.randomUUID()
    let source!: ReadableStreamDefaultController<UIMessageChunk>
    await runs.start({
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
      expect(runs.getSnapshot(conversationId)?.status).toBe('completed'),
    )

    const response = await route(runs).request(`/${conversationId}/stream`)

    expect(response.status).toBe(200)
    expect(await response.text()).toContain('"type":"text-start"')
  })

  it('streams complete panel assignments to the extension over SSE', async () => {
    const runs = new ConversationRuns()
    await runs.start({
      conversationId: crypto.randomUUID(),
      messages: [],
      panelTabIds: [42],
      createStream: () => new ReadableStream<UIMessageChunk>(),
    })

    const response = await route(runs).request(
      'http://localhost/panels',
      { headers: { Host: 'localhost' } },
      localServer,
    )
    expect(response.status).toBe(200)
    const reader = response.body?.getReader()
    const first = await reader?.read()
    const text = new TextDecoder().decode(first?.value)
    expect(text).toContain('"assignments"')
    expect(text).toContain('"tabId":42')
    await reader?.cancel()
  })

  it('cancels a run through the explicit stop endpoint', async () => {
    const runs = new ConversationRuns()
    const conversationId = crypto.randomUUID()
    await runs.start({
      conversationId,
      messages: [],
      createStream: () => new ReadableStream<UIMessageChunk>(),
    })

    const response = await route(runs).request(`/${conversationId}/stop`, {
      method: 'POST',
    })

    expect(await response.json()).toEqual({ stopped: true })
    expect(runs.getSnapshot(conversationId)?.status).toBe('aborted')
  })
})

function route(runs: ConversationRuns) {
  return createChatRoutes({
    browser: {} as never,
    browserMcp: {} as never,
    serverPort: 9000,
    conversationRuns: runs,
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
