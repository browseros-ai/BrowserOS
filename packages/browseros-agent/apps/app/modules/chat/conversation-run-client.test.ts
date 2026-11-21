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

    const controller = new AbortController()
    const state = await fetchConversationRunState(
      'http://127.0.0.1:9000',
      'conversation/one',
      controller.signal,
      fetchImpl,
    )

    const request = fetchImpl.mock.calls[0]?.[0]
    expect(request?.url).toBe(
      'http://127.0.0.1:9000/chat/conversation%2Fone/state',
    )
    expect(request?.cache).toBe('no-store')
    controller.abort()
    expect(request?.signal.aborted).toBe(true)
    expect(state?.status).toBe('running')
    expect(state?.messages[0]?.id).toBe('user-1')
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

  it('answers undefined when the server knows no run for the conversation', async () => {
    const state = await fetchConversationRunState(
      'http://127.0.0.1:9000',
      'conversation',
      undefined,
      async () =>
        Response.json({ error: 'Conversation run not found' }, { status: 404 }),
    )
    expect(state).toBeUndefined()
  })

  it('still fails on a server error so the caller can tell it from no run', async () => {
    await expect(
      fetchConversationRunState(
        'http://127.0.0.1:9000',
        'conversation',
        undefined,
        async () => Response.json({ error: 'boom' }, { status: 500 }),
      ),
    ).rejects.toThrow('500')
  })

  it('rejects malformed state instead of poisoning React chat state', async () => {
    await expect(
      fetchConversationRunState(
        'http://127.0.0.1:9000',
        'conversation',
        undefined,
        async () => Response.json({ status: 'running' }),
      ),
    ).rejects.toThrow('Invalid active conversation state')
  })
})
