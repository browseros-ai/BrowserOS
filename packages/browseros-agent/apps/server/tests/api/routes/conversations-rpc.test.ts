/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Exercises the conversations routes through the typed hc client (the same
 * contract the extension consumes), asserting the typed response shapes resolve
 * end to end. This locks the `ConversationRoutes` type export against server
 * route changes.
 */

import { describe, expect, it } from 'bun:test'
import type { UIMessage } from 'ai'
import { hc } from 'hono/client'
import { createConversationRoutes } from '../../../src/api/routes/conversations'
import type { ConversationRoutes } from '../../../src/rpc'

const ID = '00000000-0000-4000-8000-000000000001'
const MISSING = '00000000-0000-4000-8000-0000000009ff'

function memoryStore() {
  const rows = new Map<string, { id: string; messages: UIMessage[] }>([
    [ID, { id: ID, messages: [{ id: 'm1', role: 'user', parts: [] }] }],
  ])
  return {
    list: async () =>
      [...rows.values()].map((r) => ({
        id: r.id,
        lastMessagedAt: 1,
        lastUserMessage: 'hi',
      })),
    get: async (id: string) => rows.get(id) ?? null,
    delete: async (id: string) => rows.delete(id),
    insertIfAbsent: async (input: { id: string; messages: UIMessage[] }) => {
      if (rows.has(input.id)) return null
      const row = { id: input.id, messages: input.messages }
      rows.set(input.id, row)
      return row
    },
  }
}

function client() {
  const routes = createConversationRoutes({ store: memoryStore() as never })
  return hc<ConversationRoutes>('http://local', {
    fetch: (input: RequestInfo | URL, init?: RequestInit) =>
      routes.request(input as never, init),
  })
}

describe('conversations routes via typed hc', () => {
  it('lists conversations with typed summaries', async () => {
    const res = await client().index.$get()
    expect(res.status).toBe(200)
    const data = await res.json()
    expect(data.conversations[0]?.id).toBe(ID)
  })

  it('gets a conversation and narrows the found branch', async () => {
    const res = await client()[':conversationId'].$get({
      param: { conversationId: ID },
    })
    expect(res.status).toBe(200)
    const data = await res.json()
    expect('conversation' in data && data.conversation.id).toBe(ID)
  })

  it('returns 404 for a missing conversation', async () => {
    const res = await client()[':conversationId'].$get({
      param: { conversationId: MISSING },
    })
    expect(res.status).toBe(404)
  })

  it('imports (insert-if-absent) and reports imported', async () => {
    const res = await client()[':conversationId'].$put({
      param: { conversationId: '00000000-0000-4000-8000-0000000000aa' },
      json: { messages: [], lastMessagedAt: 2 },
    })
    expect(res.status).toBe(200)
    const data = await res.json()
    expect(data.imported).toBe(true)
  })

  it('deletes a conversation', async () => {
    const res = await client()[':conversationId'].$delete({
      param: { conversationId: ID },
    })
    expect(res.status).toBe(200)
  })
})
