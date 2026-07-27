import { describe, expect, it } from 'bun:test'
import type { HistoryConversation } from './types'
import { mergeHistoryConversations } from './utils'

const conv = (
  id: string,
  lastMessagedAt: number,
  lastUserMessage = id,
): HistoryConversation => ({ id, lastMessagedAt, lastUserMessage })

describe('mergeHistoryConversations', () => {
  it('unions disjoint local and remote and sorts newest first', () => {
    const local = [conv('a', 300), conv('c', 100)]
    const remote = [conv('b', 200), conv('d', 50)]

    expect(mergeHistoryConversations(local, remote).map((c) => c.id)).toEqual([
      'a',
      'b',
      'c',
      'd',
    ])
  })

  it('dedupes by id and lets the local copy win', () => {
    const local = [conv('shared', 500, 'local text')]
    const remote = [conv('shared', 999, 'remote text')]

    const merged = mergeHistoryConversations(local, remote)
    expect(merged).toHaveLength(1)
    expect(merged[0].lastUserMessage).toBe('local text')
    expect(merged[0].lastMessagedAt).toBe(500)
  })

  it('returns remote-only history when local is empty', () => {
    const remote = [conv('b', 200), conv('d', 50)]
    expect(mergeHistoryConversations([], remote).map((c) => c.id)).toEqual([
      'b',
      'd',
    ])
  })

  it('returns local-only history when remote is empty', () => {
    const local = [conv('a', 300), conv('c', 100)]
    expect(mergeHistoryConversations(local, []).map((c) => c.id)).toEqual([
      'a',
      'c',
    ])
  })

  it('returns an empty list when both are empty', () => {
    expect(mergeHistoryConversations([], [])).toEqual([])
  })
})
