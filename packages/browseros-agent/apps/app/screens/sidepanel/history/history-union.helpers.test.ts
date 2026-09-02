import { describe, expect, it } from 'bun:test'
import type {
  GroupedConversations,
  HistoryConversation,
} from './components/types'
import {
  excludeLocalConversations,
  hasAnyConversation,
} from './history-union.helpers'

function conversation(id: string): HistoryConversation {
  return { id, lastMessagedAt: 1, lastUserMessage: 'hi' }
}

function grouped(
  overrides: Partial<GroupedConversations> = {},
): GroupedConversations {
  return { today: [], thisWeek: [], thisMonth: [], older: [], ...overrides }
}

describe('excludeLocalConversations', () => {
  // One id space across extension storage, the local server and the cloud, so
  // a conversation synced before sync was turned off appears in both lists.
  it('drops a cloud conversation that also exists locally', () => {
    const result = excludeLocalConversations(
      [conversation('a'), conversation('b')],
      new Set(['a']),
    )
    expect(result.map((c) => c.id)).toEqual(['b'])
  })

  it('keeps everything when nothing is local', () => {
    const result = excludeLocalConversations(
      [conversation('a'), conversation('b')],
      new Set(),
    )
    expect(result.map((c) => c.id)).toEqual(['a', 'b'])
  })

  it('returns nothing when every cloud conversation is already local', () => {
    const result = excludeLocalConversations(
      [conversation('a')],
      new Set(['a', 'b']),
    )
    expect(result).toEqual([])
  })

  it('does not mutate the input', () => {
    const cloud = [conversation('a')]
    excludeLocalConversations(cloud, new Set(['a']))
    expect(cloud).toHaveLength(1)
  })
})

describe('hasAnyConversation', () => {
  it('is false for an empty set', () => {
    expect(hasAnyConversation(grouped())).toBe(false)
  })

  for (const bucket of ['today', 'thisWeek', 'thisMonth', 'older'] as const) {
    it(`is true when only ${bucket} has one`, () => {
      expect(
        hasAnyConversation(grouped({ [bucket]: [conversation('a')] })),
      ).toBe(true)
    })
  }
})
