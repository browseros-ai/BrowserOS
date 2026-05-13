/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Tests for conversation mutation helpers (undo / fork).
 */

import { describe, expect, it } from 'bun:test'
import {
  copyMessagesUpTo,
  findTurnEnd,
  findTurnStart,
  truncateMessages,
} from './conversation-mutations'

// ── Helpers ────────────────────────────────────────────────────────

function userMsg(id: string, text: string) {
  return { User: { id, content: [{ Text: text }] } }
}

function agentMsg(text: string) {
  return { Agent: { content: [{ Text: text }], tool_results: {} } }
}

const RESUME = 'Resume' as const

// ── findTurnStart ──────────────────────────────────────────────────

describe('findTurnStart', () => {
  it('returns 0 when index points to first user message', () => {
    const msgs = [userMsg('u1', 'hello'), agentMsg('hi')]
    expect(findTurnStart(msgs, 0)).toBe(0)
  })

  it('returns user message index when index points to agent reply', () => {
    const msgs = [userMsg('u1', 'hello'), agentMsg('hi')]
    expect(findTurnStart(msgs, 1)).toBe(0)
  })

  it('finds the correct turn start in multi-turn conversation', () => {
    const msgs = [
      userMsg('u1', 'hello'),   // 0
      agentMsg('hi'),           // 1
      userMsg('u2', 'how are'), // 2
      agentMsg('good'),         // 3
    ]
    expect(findTurnStart(msgs, 2)).toBe(2)
    expect(findTurnStart(msgs, 3)).toBe(2)
  })

  it('skips Resume markers', () => {
    const msgs = [
      userMsg('u1', 'hello'),   // 0
      agentMsg('hi'),           // 1
      RESUME,                   // 2
      userMsg('u2', 'again'),   // 3
      agentMsg('ok'),           // 4
    ]
    expect(findTurnStart(msgs, 3)).toBe(3)
    expect(findTurnStart(msgs, 4)).toBe(3)
  })

  it('returns 0 for empty messages', () => {
    expect(findTurnStart([], 0)).toBe(0)
  })
})

// ── findTurnEnd ────────────────────────────────────────────────────

describe('findTurnEnd', () => {
  it('returns same index for user-only turn', () => {
    const msgs = [userMsg('u1', 'hello')]
    expect(findTurnEnd(msgs, 0)).toBe(0)
  })

  it('includes agent reply in turn', () => {
    const msgs = [
      userMsg('u1', 'hello'),   // 0
      agentMsg('hi'),           // 1
    ]
    expect(findTurnEnd(msgs, 0)).toBe(1)
  })

  it('stops before next user message', () => {
    const msgs = [
      userMsg('u1', 'hello'),   // 0
      agentMsg('hi'),           // 1
      agentMsg('more'),         // 2
      userMsg('u2', 'next'),    // 3
    ]
    expect(findTurnEnd(msgs, 0)).toBe(2)
  })

  it('skips Resume markers within turn', () => {
    const msgs = [
      userMsg('u1', 'hello'),   // 0
      RESUME,                   // 1
      agentMsg('hi'),           // 2
    ]
    expect(findTurnEnd(msgs, 0)).toBe(2)
  })
})

// ── truncateMessages (undo) ───────────────────────────────────────

describe('truncateMessages', () => {
  it('removes a turn from the end', () => {
    const msgs = [
      userMsg('u1', 'hello'),   // 0
      agentMsg('hi'),           // 1
      userMsg('u2', 'remove'),  // 2
      agentMsg('bye'),          // 3
    ]
    const result = truncateMessages(msgs, 2)
    expect(result).toHaveLength(2)
    expect(result).toEqual([msgs[0], msgs[1]])
  })

  it('removes all messages when undoing first turn', () => {
    const msgs = [
      userMsg('u1', 'hello'),
      agentMsg('hi'),
    ]
    const result = truncateMessages(msgs, 0)
    expect(result).toHaveLength(0)
  })

  it('undoes from agent message by finding its turn start', () => {
    const msgs = [
      userMsg('u1', 'keep'),    // 0
      agentMsg('yes'),          // 1
      userMsg('u2', 'remove'),  // 2
      agentMsg('bye'),          // 3
    ]
    const result = truncateMessages(msgs, 3)
    expect(result).toHaveLength(2)
    expect(result).toEqual([msgs[0], msgs[1]])
  })

  it('handles single turn', () => {
    const msgs = [userMsg('u1', 'hello'), agentMsg('hi')]
    const result = truncateMessages(msgs, 1)
    expect(result).toHaveLength(0)
  })

  it('preserves earlier turns when undoing middle turn', () => {
    const msgs = [
      userMsg('u1', 'a'),    // 0
      agentMsg('b'),         // 1
      userMsg('u2', 'c'),    // 2
      agentMsg('d'),         // 3
      userMsg('u3', 'e'),    // 4
      agentMsg('f'),         // 5
    ]
    const result = truncateMessages(msgs, 2)
    expect(result).toHaveLength(2)
    expect(result[0]).toEqual(msgs[0])
  })
})

// ── copyMessagesUpTo (fork) ───────────────────────────────────────

describe('copyMessagesUpTo', () => {
  it('copies up to and including the turn', () => {
    const msgs = [
      userMsg('u1', 'hello'),   // 0
      agentMsg('hi'),           // 1
      userMsg('u2', 'fork'),    // 2
      agentMsg('here'),         // 3
    ]
    const result = copyMessagesUpTo(msgs, 2)
    expect(result).toHaveLength(4) // both turns included
  })

  it('copies only first turn when forking at index 0', () => {
    const msgs = [
      userMsg('u1', 'hello'),   // 0
      agentMsg('hi'),           // 1
      userMsg('u2', 'next'),    // 2
      agentMsg('ok'),           // 3
    ]
    const result = copyMessagesUpTo(msgs, 0)
    expect(result).toHaveLength(2)
  })

  it('includes agent reply in copied turn', () => {
    const msgs = [
      userMsg('u1', 'hello'),   // 0
      agentMsg('hi'),           // 1
    ]
    const result = copyMessagesUpTo(msgs, 0)
    expect(result).toHaveLength(2)
    expect(result[1]).toEqual(msgs[1])
  })

  it('handles forking from agent message index', () => {
    const msgs = [
      userMsg('u1', 'a'),    // 0
      agentMsg('b'),         // 1
      userMsg('u2', 'c'),    // 2
      agentMsg('d'),         // 3
    ]
    const result = copyMessagesUpTo(msgs, 1)
    // findTurnStart(msgs, 1) = 0, findTurnEnd(msgs, 0) = 1
    expect(result).toHaveLength(2)
  })
})
