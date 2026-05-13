import { describe, expect, it } from 'bun:test'
import type { UIMessage } from 'ai'
import {
  editMessageAtTurn,
  editMessageById,
  findTurnEndIndex,
  findTurnStartIndex,
  forkFromMessage,
  forkFromMessageId,
  undoFromMessage,
  undoFromMessageId,
} from './index'

// ═══════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════

let idCounter = 0
function nextId(): string {
  return `msg-${++idCounter}`
}

function userMsg(text: string): UIMessage {
  return { id: nextId(), role: 'user', parts: [{ type: 'text', text }] }
}

function assistantMsg(text: string): UIMessage {
  return { id: nextId(), role: 'assistant', parts: [{ type: 'text', text }] }
}

/** Build a typical multi-turn conversation: U A U A U A */
function buildConversation(): UIMessage[] {
  return [
    userMsg('Hello'),
    assistantMsg('Hi there!'),
    userMsg('How are you?'),
    assistantMsg('I am doing well!'),
    userMsg('Tell me a joke'),
    assistantMsg('Why did the chicken cross the road?'),
  ]
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

describe('findTurnStartIndex', () => {
  it('returns 0 for the first user message', () => {
    const msgs = buildConversation()
    expect(findTurnStartIndex(msgs, 0)).toBe(0)
  })

  it('returns the user message index when given an assistant message', () => {
    const msgs = buildConversation()
    // Index 1 is assistant "Hi there!" — turn starts at 0
    expect(findTurnStartIndex(msgs, 1)).toBe(0)
  })

  it('returns correct user index for second turn', () => {
    const msgs = buildConversation()
    // Index 2 is user "How are you?"
    expect(findTurnStartIndex(msgs, 2)).toBe(2)
    // Index 3 is assistant "I am doing well!"
    expect(findTurnStartIndex(msgs, 3)).toBe(2)
  })

  it('returns correct user index for third turn', () => {
    const msgs = buildConversation()
    expect(findTurnStartIndex(msgs, 4)).toBe(4)
    expect(findTurnStartIndex(msgs, 5)).toBe(4)
  })

  it('returns -1 for out-of-bounds index', () => {
    const msgs = buildConversation()
    expect(findTurnStartIndex(msgs, -1)).toBe(-1)
    expect(findTurnStartIndex(msgs, 100)).toBe(-1)
  })

  it('returns -1 for empty messages', () => {
    expect(findTurnStartIndex([], 0)).toBe(-1)
  })
})

describe('findTurnEndIndex', () => {
  it('returns user message index when no assistant follows', () => {
    const msgs = [userMsg('Hello')]
    expect(findTurnEndIndex(msgs, 0)).toBe(0)
  })

  it('returns last consecutive assistant message', () => {
    const msgs = buildConversation()
    // Turn at index 0: user + assistant(1)
    expect(findTurnEndIndex(msgs, 0)).toBe(1)
    // Turn at index 2: user + assistant(3)
    expect(findTurnEndIndex(msgs, 2)).toBe(3)
  })

  it('handles out-of-bounds', () => {
    expect(findTurnEndIndex([], 0)).toBe(0)
    expect(findTurnEndIndex(buildConversation(), -1)).toBe(-1)
  })
})

describe('undoFromMessage', () => {
  it('undoes the first turn (removes everything)', () => {
    const msgs = buildConversation()
    const result = undoFromMessage(msgs, 0)
    expect(result).toHaveLength(0)
  })

  it('undoes from assistant message in first turn', () => {
    const msgs = buildConversation()
    const result = undoFromMessage(msgs, 1)
    expect(result).toHaveLength(0)
  })

  it('undoes the second turn, keeps first turn', () => {
    const msgs = buildConversation()
    const result = undoFromMessage(msgs, 2)
    expect(result).toHaveLength(2)
    expect(result[0].parts[0]).toEqual({ type: 'text', text: 'Hello' })
    expect(result[1].parts[0]).toEqual({ type: 'text', text: 'Hi there!' })
  })

  it('undoes from assistant in second turn, keeps first turn', () => {
    const msgs = buildConversation()
    const result = undoFromMessage(msgs, 3)
    expect(result).toHaveLength(2)
  })

  it('undoes the third turn, keeps first two turns', () => {
    const msgs = buildConversation()
    const result = undoFromMessage(msgs, 4)
    expect(result).toHaveLength(4)
  })

  it('undoes from last assistant message', () => {
    const msgs = buildConversation()
    const result = undoFromMessage(msgs, 5)
    expect(result).toHaveLength(4)
  })

  it('returns same array for out-of-bounds', () => {
    const msgs = buildConversation()
    expect(undoFromMessage(msgs, -1)).toBe(msgs)
    expect(undoFromMessage(msgs, 100)).toBe(msgs)
  })

  it('returns empty for empty messages', () => {
    // edge case: -1 triggers fallback to same array
    expect(undoFromMessage([], 0)).toEqual([])
  })
})

describe('forkFromMessage', () => {
  it('forks from first turn — returns empty prefix', () => {
    const msgs = buildConversation()
    const result = forkFromMessage(msgs, 0)
    expect(result).toHaveLength(0)
  })

  it('forks from second turn — keeps first turn', () => {
    const msgs = buildConversation()
    const result = forkFromMessage(msgs, 2)
    expect(result).toHaveLength(2)
    expect(result[0].parts[0]).toEqual({ type: 'text', text: 'Hello' })
    expect(result[1].parts[0]).toEqual({ type: 'text', text: 'Hi there!' })
  })

  it('forks from assistant in second turn — same as forking from that turn', () => {
    const msgs = buildConversation()
    const result = forkFromMessage(msgs, 3)
    expect(result).toHaveLength(2)
  })

  it('forks from third turn — keeps first two turns', () => {
    const msgs = buildConversation()
    const result = forkFromMessage(msgs, 4)
    expect(result).toHaveLength(4)
  })

  it('returns empty for empty messages', () => {
    expect(forkFromMessage([], 0)).toEqual([])
  })
})

describe('editMessageAtTurn', () => {
  it('edits first user message, truncates rest', () => {
    const msgs = buildConversation()
    const result = editMessageAtTurn(msgs, 0, 'Goodbye')
    expect(result).toHaveLength(1)
    expect(result[0].parts[0]).toEqual({ type: 'text', text: 'Goodbye' })
    // Preserves the original message id
    expect(result[0].id).toBe(msgs[0].id)
  })

  it('edits from assistant message index — edits the user msg of that turn', () => {
    const msgs = buildConversation()
    const result = editMessageAtTurn(msgs, 3, 'Updated question')
    expect(result).toHaveLength(3)
    // First turn untouched
    expect(result[0].parts[0]).toEqual({ type: 'text', text: 'Hello' })
    expect(result[1].parts[0]).toEqual({ type: 'text', text: 'Hi there!' })
    // Second turn user message edited
    expect(result[2].parts[0]).toEqual({ type: 'text', text: 'Updated question' })
  })

  it('edits third turn, keeps first two turns', () => {
    const msgs = buildConversation()
    const result = editMessageAtTurn(msgs, 4, 'New joke request')
    expect(result).toHaveLength(5)
    expect(result[4].parts[0]).toEqual({ type: 'text', text: 'New joke request' })
  })

  it('returns same array for out-of-bounds', () => {
    const msgs = buildConversation()
    expect(editMessageAtTurn(msgs, -1, 'test')).toBe(msgs)
    expect(editMessageAtTurn(msgs, 100, 'test')).toBe(msgs)
  })

  it('preserves other properties of the original message', () => {
    const msgs = buildConversation()
    const result = editMessageAtTurn(msgs, 0, 'Edited')
    expect(result[0].role).toBe('user')
    expect(result[0].id).toBe(msgs[0].id)
  })
})

// ═══════════════════════════════════════════════════════════════════════════
// ID-based function tests
// ═══════════════════════════════════════════════════════════════════════════

describe('undoFromMessageId', () => {
  it('undoes by message ID', () => {
    const msgs = buildConversation()
    // Undo from the 3rd turn's user message
    const result = undoFromMessageId(msgs, msgs[4].id)
    expect(result).toHaveLength(4)
  })

  it('returns same array for unknown ID', () => {
    const msgs = buildConversation()
    expect(undoFromMessageId(msgs, 'nonexistent')).toBe(msgs)
  })
})

describe('forkFromMessageId', () => {
  it('forks by message ID', () => {
    const msgs = buildConversation()
    const result = forkFromMessageId(msgs, msgs[4].id)
    expect(result).not.toBeNull()
    expect(result!.messages).toHaveLength(4)
  })

  it('returns null for first turn fork', () => {
    const msgs = buildConversation()
    const result = forkFromMessageId(msgs, msgs[0].id)
    expect(result).toBeNull()
  })

  it('returns null for unknown ID', () => {
    const msgs = buildConversation()
    const result = forkFromMessageId(msgs, 'nonexistent')
    expect(result).toBeNull()
  })
})

describe('editMessageById', () => {
  it('edits by message ID', () => {
    const msgs = buildConversation()
    const result = editMessageById(msgs, msgs[2].id, 'Updated')
    expect(result).not.toBeNull()
    expect(result!).toHaveLength(3)
    expect(result![2].parts[0]).toEqual({ type: 'text', text: 'Updated' })
  })

  it('returns null for unknown ID', () => {
    const msgs = buildConversation()
    const result = editMessageById(msgs, 'nonexistent', 'test')
    expect(result).toBeNull()
  })
})
