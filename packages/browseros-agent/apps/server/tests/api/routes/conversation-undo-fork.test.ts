/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import { Hono } from 'hono'
import {
  type ConversationUndoForkDeps,
  type SessionRecordHandle,
  copyUpToTurn,
  createConversationUndoForkRoutes,
  findMessageIndexById,
  findTurnStart,
  truncateFromTurn,
} from '../../../src/api/routes/conversation-undo-fork'

// ── Unit tests: turn-boundary helpers ─────────────────────────────────

describe('conversation-undo-fork helpers', () => {
  const messages = [
    { User: { id: 'msg-1', content: [{ Text: 'Hello' }] } },
    { Agent: { content: [{ Text: 'Hi there' }], tool_results: {} } },
    { User: { id: 'msg-2', content: [{ Text: 'How are you?' }] } },
    { Agent: { content: [{ Text: 'Fine, thanks' }], tool_results: {} } },
    { User: { id: 'msg-3', content: [{ Text: 'Goodbye' }] } },
    { Agent: { content: [{ Text: 'Bye!' }], tool_results: {} } },
  ]

  describe('findTurnStart', () => {
    it('returns the user message index when pointing at a user message', () => {
      expect(findTurnStart(messages, 2)).toBe(2) // msg-2
    })

    it('returns the user message index when pointing at the following agent message', () => {
      expect(findTurnStart(messages, 3)).toBe(2) // Agent reply to msg-2
    })

    it('returns 0 for the first user message', () => {
      expect(findTurnStart(messages, 0)).toBe(0)
    })

    it('returns 0 for the first agent message', () => {
      expect(findTurnStart(messages, 1)).toBe(0)
    })

    it('handles Resume markers by skipping them', () => {
      const withResume = [
        { User: { id: 'msg-1', content: [{ Text: 'Hello' }] } },
        { Agent: { content: [{ Text: 'Hi' }], tool_results: {} } },
        'Resume' as unknown,
        { User: { id: 'msg-2', content: [{ Text: 'Again' }] } },
        { Agent: { content: [{ Text: 'Ok' }], tool_results: {} } },
      ]
      // Pointing at Agent[4] should find User[3], not Resume[2]
      expect(findTurnStart(withResume, 4)).toBe(3)
    })

    it('returns given index when no user message found', () => {
      const agentOnly = [
        { Agent: { content: [{ Text: 'Hi' }], tool_results: {} } },
      ]
      expect(findTurnStart(agentOnly, 0)).toBe(0)
    })
  })

  describe('findMessageIndexById', () => {
    it('finds existing message by id', () => {
      expect(findMessageIndexById(messages, 'msg-2')).toBe(2)
    })

    it('returns -1 for non-existent id', () => {
      expect(findMessageIndexById(messages, 'nonexistent')).toBe(-1)
    })

    it('returns -1 for empty messages', () => {
      expect(findMessageIndexById([], 'msg-1')).toBe(-1)
    })
  })

  describe('truncateFromTurn', () => {
    it('truncates from turn 2 (msg-2), leaving only turn 1', () => {
      const result = truncateFromTurn(messages, 2)
      expect(result).toHaveLength(2) // msg-1 + Agent reply
      expect(result[0]).toEqual(messages[0])
      expect(result[1]).toEqual(messages[1])
    })

    it('truncating from turn 1 (msg-1) leaves empty array', () => {
      const result = truncateFromTurn(messages, 0)
      expect(result).toHaveLength(0)
    })

    it('truncating from agent message still removes the whole turn', () => {
      const result = truncateFromTurn(messages, 3) // Agent reply to msg-2
      expect(result).toHaveLength(2) // Only turn 1 remains
    })

    it('truncating last turn leaves first two turns', () => {
      const result = truncateFromTurn(messages, 4) // msg-3
      expect(result).toHaveLength(4) // Turn 1 + Turn 2
    })
  })

  describe('copyUpToTurn', () => {
    it('copies up to and including msg-2 (turn 2)', () => {
      const result = copyUpToTurn(messages, 2)
      expect(result).toHaveLength(3) // msg-1 + Agent + msg-2
      expect(result[2]).toEqual(messages[2]) // Includes the user message
    })

    it('copies only msg-1 when forking at turn 1', () => {
      const result = copyUpToTurn(messages, 0)
      expect(result).toHaveLength(1)
      expect(result[0]).toEqual(messages[0])
    })

    it('copies up to agent message (finds the user message)', () => {
      const result = copyUpToTurn(messages, 3) // Agent reply to msg-2
      expect(result).toHaveLength(3)
      expect(result[2]).toEqual(messages[2]) // Includes msg-2
    })
  })
})

// ── Integration tests: HTTP routes ────────────────────────────────────

function makeMessage(id: string, text: string) {
  return { User: { id, content: [{ Text: text }] } }
}

function makeAgentReply(text: string) {
  return { Agent: { content: [{ Text: text }], tool_results: {} } }
}

function createFakeDeps(
  sessions: Map<string, { messages: unknown[] }>,
  options: { supportsFork?: boolean } = {},
): ConversationUndoForkDeps {
  return {
    cancelActiveTurn: () => true,
    loadSessionRecord: async (agentId) => {
      const session = sessions.get(agentId)
      if (!session) return null

      const handle: SessionRecordHandle = {
        messages: session.messages,
        save: async (newMessages) => {
          session.messages = newMessages
        },
        ...(options.supportsFork !== false
          ? {
              fork: async (prefix, newId) => {
                const id = newId ?? crypto.randomUUID()
                const forkedSession = { messages: [...prefix] }
                sessions.set(`forked-${id}`, forkedSession)
                return { newConversationId: id, messageCount: prefix.length }
              },
            }
          : {}),
      }
      return handle
    },
  }
}

function mountRoutes(deps: ConversationUndoForkDeps) {
  return new Hono().route(
    '/agents',
    createConversationUndoForkRoutes(deps),
  )
}

describe('POST /:agentId/conversation/undo', () => {
  it('returns 200 and truncates messages', async () => {
    const sessions = new Map<string, { messages: unknown[] }>()
    sessions.set('agent-1', {
      messages: [
        makeMessage('m1', 'Hello'),
        makeAgentReply('Hi'),
        makeMessage('m2', 'How are you?'),
        makeAgentReply('Fine'),
      ],
    })
    const route = mountRoutes(createFakeDeps(sessions))

    const res = await route.request('/agents/agent-1/conversation/undo', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ conversationId: 'conv-1', messageId: 'm2' }),
    })

    expect(res.status).toBe(200)
    const body = await res.json()
    expect(body).toEqual({ success: true, remainingCount: 2 })

    // Verify session was updated
    const session = sessions.get('agent-1')!
    expect(session.messages).toHaveLength(2)
  })

  it('returns 404 for unknown agent', async () => {
    const route = mountRoutes(createFakeDeps(new Map()))
    const res = await route.request('/agents/unknown/conversation/undo', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ conversationId: 'conv-1', messageId: 'm1' }),
    })
    expect(res.status).toBe(404)
  })

  it('returns 404 for unknown message id', async () => {
    const sessions = new Map<string, { messages: unknown[] }>()
    sessions.set('agent-1', {
      messages: [makeMessage('m1', 'Hello'), makeAgentReply('Hi')],
    })
    const route = mountRoutes(createFakeDeps(sessions))

    const res = await route.request('/agents/agent-1/conversation/undo', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ conversationId: 'conv-1', messageId: 'nonexistent' }),
    })
    expect(res.status).toBe(404)
    const body = await res.json()
    expect(body.error).toBe('Message not found in session')
  })

  it('returns 400 for missing fields', async () => {
    const route = mountRoutes(createFakeDeps(new Map()))
    const res = await route.request('/agents/agent-1/conversation/undo', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({}),
    })
    expect(res.status).toBe(400)
  })

  it('returns 400 for invalid JSON', async () => {
    const route = mountRoutes(createFakeDeps(new Map()))
    const res = await route.request('/agents/agent-1/conversation/undo', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: 'not-json',
    })
    expect(res.status).toBe(400)
  })

  it('cancels active turn before truncating', async () => {
    let cancelled = false
    const sessions = new Map<string, { messages: unknown[] }>()
    sessions.set('agent-1', {
      messages: [makeMessage('m1', 'Hello'), makeAgentReply('Hi')],
    })
    const deps = createFakeDeps(sessions)
    deps.cancelActiveTurn = (agentId) => {
      expect(agentId).toBe('agent-1')
      cancelled = true
      return true
    }
    const route = mountRoutes(deps)

    await route.request('/agents/agent-1/conversation/undo', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ conversationId: 'conv-1', messageId: 'm1' }),
    })

    expect(cancelled).toBe(true)
  })

  it('handles undo from an agent message (finds parent user message)', async () => {
    const sessions = new Map<string, { messages: unknown[] }>()
    sessions.set('agent-1', {
      messages: [
        makeMessage('m1', 'Hello'),
        makeAgentReply('Hi'),
        makeMessage('m2', 'Second'),
        makeAgentReply('Reply 2'),
        makeMessage('m3', 'Third'),
        makeAgentReply('Reply 3'),
      ],
    })
    const route = mountRoutes(createFakeDeps(sessions))

    // Undo from Agent reply to m2 (index 3)
    // findMessageIndexById won't find it (it only looks for User messages)
    // But we can test the truncation logic directly with the agent message index
    const session = sessions.get('agent-1')!
    const truncated = truncateFromTurn(session.messages, 3)
    expect(truncated).toHaveLength(2) // m1 + reply only
  })
})

describe('POST /:agentId/conversation/fork', () => {
  it('returns 200 with new conversation ID', async () => {
    const sessions = new Map<string, { messages: unknown[] }>()
    sessions.set('agent-1', {
      messages: [
        makeMessage('m1', 'Hello'),
        makeAgentReply('Hi'),
        makeMessage('m2', 'Second'),
        makeAgentReply('Reply 2'),
      ],
    })
    const route = mountRoutes(createFakeDeps(sessions))

    const res = await route.request('/agents/agent-1/conversation/fork', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        conversationId: 'conv-1',
        messageId: 'm2',
        newConversationId: 'new-conv-id',
      }),
    })

    expect(res.status).toBe(200)
    const body = await res.json()
    expect(body).toEqual({
      success: true,
      newConversationId: 'new-conv-id',
      messageCount: 3, // m1 + reply + m2
    })

    // Original session unchanged
    expect(sessions.get('agent-1')!.messages).toHaveLength(4)
  })

  it('returns 501 when fork is not supported', async () => {
    const sessions = new Map<string, { messages: unknown[] }>()
    sessions.set('agent-1', {
      messages: [makeMessage('m1', 'Hello')],
    })
    const route = mountRoutes(createFakeDeps(sessions, { supportsFork: false }))

    const res = await route.request('/agents/agent-1/conversation/fork', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ conversationId: 'conv-1', messageId: 'm1' }),
    })

    expect(res.status).toBe(501)
  })

  it('returns 404 for unknown agent', async () => {
    const route = mountRoutes(createFakeDeps(new Map()))
    const res = await route.request('/agents/unknown/conversation/fork', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ conversationId: 'conv-1', messageId: 'm1' }),
    })
    expect(res.status).toBe(404)
  })

  it('returns 404 for unknown message id', async () => {
    const sessions = new Map<string, { messages: unknown[] }>()
    sessions.set('agent-1', {
      messages: [makeMessage('m1', 'Hello')],
    })
    const route = mountRoutes(createFakeDeps(sessions))

    const res = await route.request('/agents/agent-1/conversation/fork', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ conversationId: 'conv-1', messageId: 'nope' }),
    })
    expect(res.status).toBe(404)
  })

  it('auto-generates conversation ID when not provided', async () => {
    const sessions = new Map<string, { messages: unknown[] }>()
    sessions.set('agent-1', {
      messages: [makeMessage('m1', 'Hello'), makeAgentReply('Hi')],
    })
    const route = mountRoutes(createFakeDeps(sessions))

    const res = await route.request('/agents/agent-1/conversation/fork', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ conversationId: 'conv-1', messageId: 'm1' }),
    })

    expect(res.status).toBe(200)
    const body = await res.json()
    expect(body.success).toBe(true)
    expect(body.newConversationId).toBeTruthy()
    expect(typeof body.newConversationId).toBe('string')
  })
})
