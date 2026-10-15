import { describe, expect, it, mock } from 'bun:test'
import {
  conversationReconnectUrl,
  fetchConversationRunState,
} from './conversation-run-client'

describe('conversation run client', () => {
  it('loads the server-owned base messages for a panel', async () => {
    const fetchImpl = mock(async (_request: Request) =>
      Response.json({
        conversationId: 'conversation/one',
        runId: 'run-1',
        status: 'running',
        messages: [{ id: 'user-1', role: 'user', parts: [] }],
      }),
    )

    // Bun 1.3 drops Request.cache even when passed no-store. Observe the real
    // constructor input so this checks the browser contract, not Bun's accessor.
    const NativeRequest = globalThis.Request
    let cache: RequestCache | undefined
    globalThis.Request = class extends NativeRequest {
      constructor(input: RequestInfo | URL, init?: RequestInit) {
        cache = init?.cache
        super(input, init)
      }
    }
    let state: Awaited<ReturnType<typeof fetchConversationRunState>>
    try {
      state = await fetchConversationRunState(
        'http://127.0.0.1:9000',
        'conversation/one',
        fetchImpl as unknown as typeof fetch,
      )
    } finally {
      globalThis.Request = NativeRequest
    }

    const request = fetchImpl.mock.calls[0]?.[0]
    expect(request?.url).toBe(
      'http://127.0.0.1:9000/chat/conversation%2Fone/state',
    )
    expect(cache).toBe('no-store')
    expect(state.status).toBe('running')
    expect(state.messages[0]?.id).toBe('user-1')
  })

  it('builds the AI SDK reconnect endpoint from the conversation id', () => {
    expect(
      conversationReconnectUrl('http://127.0.0.1:9000', 'conversation/one'),
    ).toBe('http://127.0.0.1:9000/chat/conversation%2Fone/stream')
  })

  it('pins reconnects to the hydrated run instead of whichever turn is newest', () => {
    expect(
      conversationReconnectUrl('http://localhost:9000', 'chat', 'run/new'),
    ).toBe('http://localhost:9000/chat/chat/stream?runId=run%2Fnew')
  })

  it('rejects malformed state instead of poisoning React chat state', async () => {
    await expect(
      fetchConversationRunState(
        'http://127.0.0.1:9000',
        'conversation',
        (async () =>
          Response.json({ status: 'running' })) as unknown as typeof fetch,
      ),
    ).rejects.toThrow('Invalid active conversation state')
  })
})
