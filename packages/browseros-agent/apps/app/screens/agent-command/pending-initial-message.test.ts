import { afterEach, describe, expect, it } from 'bun:test'
import type { StagedAttachment } from '@/lib/attachments'
import {
  consumePendingInitialMessage,
  peekPendingInitialMessage,
  setPendingInitialMessage,
} from './pending-initial-message'

function makeAttachment(id: string): StagedAttachment {
  return {
    id,
    kind: 'image',
    mediaType: 'image/png',
    name: `${id}.png`,
    dataUrl: `data:image/png;base64,${id}`,
    payload: {
      kind: 'image',
      mediaType: 'image/png',
      name: `${id}.png`,
      dataUrl: `data:image/png;base64,${id}`,
    },
  }
}

afterEach(() => {
  // Drain any leftover pending entry so tests don't leak into each
  // other (the module-scope state survives across `it` blocks).
  consumePendingInitialMessage('drain', 'session-drain')
  // If still set, clear by consuming with the matching id.
  const leftover = peekPendingInitialMessage()
  if (leftover)
    consumePendingInitialMessage(leftover.agentId, leftover.sessionId)
})

describe('pending-initial-message', () => {
  it('consume returns the payload set for the same agentId', () => {
    setPendingInitialMessage({
      agentId: 'agent-a',
      sessionId: 'session-a',
      text: 'hello',
      attachments: [makeAttachment('one')],
      createdAt: Date.now(),
    })
    const result = consumePendingInitialMessage('agent-a', 'session-a')
    expect(result?.text).toBe('hello')
    expect(result?.attachments).toHaveLength(1)
    expect(result?.attachments[0]?.id).toBe('one')
  })

  it('consume is destructive — second call returns null', () => {
    setPendingInitialMessage({
      agentId: 'agent-a',
      sessionId: 'session-a',
      text: 'hello',
      attachments: [],
      createdAt: Date.now(),
    })
    expect(consumePendingInitialMessage('agent-a', 'session-a')).not.toBeNull()
    expect(consumePendingInitialMessage('agent-a', 'session-a')).toBeNull()
  })

  it('consume returns null and preserves entry when agentId or sessionId differs', () => {
    setPendingInitialMessage({
      agentId: 'agent-a',
      sessionId: 'session-a',
      text: 'hello',
      attachments: [],
      createdAt: Date.now(),
    })
    expect(consumePendingInitialMessage('agent-b', 'session-a')).toBeNull()
    expect(consumePendingInitialMessage('agent-a', 'session-b')).toBeNull()
    expect(peekPendingInitialMessage()?.agentId).toBe('agent-a')
    expect(consumePendingInitialMessage('agent-a', 'session-a')).not.toBeNull()
  })

  it('returns null for entries older than the TTL', () => {
    setPendingInitialMessage({
      agentId: 'agent-a',
      sessionId: 'session-a',
      text: 'old',
      attachments: [],
      createdAt: Date.now() - 11_000, // older than 10 s TTL
    })
    expect(consumePendingInitialMessage('agent-a', 'session-a')).toBeNull()
  })

  it('replaces a previous pending entry when set is called again', () => {
    setPendingInitialMessage({
      agentId: 'agent-a',
      sessionId: 'session-a',
      text: 'first',
      attachments: [],
      createdAt: Date.now(),
    })
    setPendingInitialMessage({
      agentId: 'agent-b',
      sessionId: 'session-b',
      text: 'second',
      attachments: [makeAttachment('two')],
      createdAt: Date.now(),
    })
    expect(consumePendingInitialMessage('agent-a', 'session-a')).toBeNull()
    const result = consumePendingInitialMessage('agent-b', 'session-b')
    expect(result?.text).toBe('second')
    expect(result?.attachments[0]?.id).toBe('two')
  })

  it('no-ops when set is called with empty agentId', () => {
    setPendingInitialMessage({
      agentId: '',
      sessionId: 'session-a',
      text: 'oops',
      attachments: [],
      createdAt: Date.now(),
    })
    expect(peekPendingInitialMessage()).toBeNull()
  })

  it('no-ops when set is called with empty sessionId', () => {
    setPendingInitialMessage({
      agentId: 'agent-a',
      sessionId: '',
      text: 'oops',
      attachments: [],
      createdAt: Date.now(),
    })
    expect(peekPendingInitialMessage()).toBeNull()
  })
})
