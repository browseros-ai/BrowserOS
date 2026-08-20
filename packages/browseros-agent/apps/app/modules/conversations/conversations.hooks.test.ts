import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'

mock.module('@/modules/browseros/agent-server-url.helpers', () => ({
  resolveAgentServerUrlWithRetry: mock(async () => 'http://127.0.0.1:9999'),
}))

const removeExecutionHistory = mock(async () => {})
mock.module('@/lib/execution-history/storage', () => ({
  removeConversationExecutionHistory: removeExecutionHistory,
}))

const {
  fetchServerConversations,
  fetchServerConversation,
  deleteServerConversation,
} = await import('./conversations.hooks')

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  })
}

const originalFetch = globalThis.fetch

describe('server conversations api', () => {
  beforeEach(() => {
    removeExecutionHistory.mockClear()
  })

  afterEach(() => {
    globalThis.fetch = originalFetch
  })

  it('lists conversations, defaulting a missing snippet to empty', async () => {
    const fetchMock = mock((_input: RequestInfo | URL) =>
      Promise.resolve(
        jsonResponse({
          conversations: [
            { id: 'a', lastMessagedAt: 2, lastUserMessage: 'hi' },
            { id: 'b', lastMessagedAt: 1 },
          ],
        }),
      ),
    )
    globalThis.fetch = fetchMock as never

    expect(await fetchServerConversations()).toEqual([
      { id: 'a', lastMessagedAt: 2, lastUserMessage: 'hi' },
      { id: 'b', lastMessagedAt: 1, lastUserMessage: '' },
    ])
    expect(fetchMock.mock.calls[0]?.[0]).toBe(
      'http://127.0.0.1:9999/conversations',
    )
  })

  it('loads a conversation and returns null on 404', async () => {
    globalThis.fetch = mock(async () =>
      jsonResponse({
        conversation: {
          id: 'a',
          messages: [{ id: 'm', role: 'user', parts: [] }],
        },
      }),
    ) as never
    expect((await fetchServerConversation('a'))?.messages).toHaveLength(1)

    globalThis.fetch = mock(async () =>
      jsonResponse({ error: 'Unknown conversation' }, 404),
    ) as never
    expect(await fetchServerConversation('missing')).toBeNull()
  })

  it('deletes then clears execution history, tolerating a 404', async () => {
    globalThis.fetch = mock(async () =>
      jsonResponse({ success: true }),
    ) as never
    await deleteServerConversation('a')
    expect(removeExecutionHistory).toHaveBeenCalledWith('a')

    removeExecutionHistory.mockClear()
    globalThis.fetch = mock(async () =>
      jsonResponse({ error: 'Unknown conversation' }, 404),
    ) as never
    await deleteServerConversation('missing')
    expect(removeExecutionHistory).toHaveBeenCalledWith('missing')
  })

  it('throws when the list request fails', async () => {
    globalThis.fetch = mock(async () =>
      jsonResponse({ error: 'boom' }, 500),
    ) as never
    await expect(fetchServerConversations()).rejects.toThrow(
      'Failed to load conversations (500)',
    )
  })
})
