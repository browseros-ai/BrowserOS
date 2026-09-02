import { describe, expect, it } from 'bun:test'
import type {
  GroupedConversations,
  HistoryConversation,
} from './components/types'
import {
  excludeLocalConversations,
  hasAnyConversation,
  shouldAdvanceCloudPage,
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

describe('shouldAdvanceCloudPage', () => {
  const stalled = {
    hasVisibleConversations: false,
    hasNextPage: true,
    isFetchingNextPage: false,
    isLoading: false,
  }

  // The page that stalls is the ordinary one right after this ships: the most
  // recent conversations exist in both stores and sort onto the first page, so
  // it deduplicates away to nothing and the sentinel never mounts to pull the
  // cloud-only conversations behind it.
  it('advances when a page deduplicates away to nothing', () => {
    expect(shouldAdvanceCloudPage(stalled)).toBe(true)
  })

  it('stops once something is visible, leaving the sentinel to take over', () => {
    expect(
      shouldAdvanceCloudPage({ ...stalled, hasVisibleConversations: true }),
    ).toBe(false)
  })

  it('terminates when the pages run out', () => {
    expect(shouldAdvanceCloudPage({ ...stalled, hasNextPage: false })).toBe(
      false,
    )
  })

  // Without these the effect would queue a second fetch on every render while
  // the first is still in flight.
  it('does not stack a fetch on top of one in flight', () => {
    expect(
      shouldAdvanceCloudPage({ ...stalled, isFetchingNextPage: true }),
    ).toBe(false)
  })

  it('waits for the first page before advancing', () => {
    expect(shouldAdvanceCloudPage({ ...stalled, isLoading: true })).toBe(false)
  })
})
