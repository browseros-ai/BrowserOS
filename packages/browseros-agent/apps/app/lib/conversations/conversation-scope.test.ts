import { describe, expect, it } from 'bun:test'
import {
  backfillConversationOwners,
  filterConversationsByOwner,
  resolveEffectiveOwnerId,
  selectUploadableConversations,
  stampConversationOwner,
} from './conversation-scope'
import type { Conversation } from './conversationStorage'

const conv = (id: string, owner?: string): Conversation => ({
  id,
  owner,
  messages: [],
  lastMessagedAt: 0,
})

describe('resolveEffectiveOwnerId', () => {
  it('uses the current user when signed in', () => {
    expect(resolveEffectiveOwnerId('A', 'B')).toBe('A')
  })

  it('falls back to the last signed-in user when signed out', () => {
    expect(resolveEffectiveOwnerId(undefined, 'A')).toBe('A')
  })

  it('is undefined when signed out and never signed in', () => {
    expect(resolveEffectiveOwnerId(undefined, null)).toBeUndefined()
  })
})

describe('filterConversationsByOwner', () => {
  const all = [conv('a', 'A'), conv('b', 'B'), conv('anon')]

  it('keeps only the effective owner and never another account', () => {
    expect(filterConversationsByOwner(all, 'A').map((c) => c.id)).toEqual(['a'])
    expect(filterConversationsByOwner(all, 'B').map((c) => c.id)).toEqual(['b'])
  })

  it('keeps only anonymous conversations when there is no effective owner', () => {
    expect(filterConversationsByOwner(all, undefined).map((c) => c.id)).toEqual(
      ['anon'],
    )
  })
})

describe('backfillConversationOwners', () => {
  it('stamps the owner onto pre-owner conversations and marks them local-only', () => {
    const result = backfillConversationOwners([conv('a'), conv('b')], 'A')
    expect(result.changed).toBe(true)
    expect(result.conversations.map((c) => c.owner)).toEqual(['A', 'A'])
    expect(result.conversations.map((c) => c.localOnly)).toEqual([true, true])
  })

  it('leaves already-owned conversations untouched and not local-only', () => {
    const result = backfillConversationOwners([conv('a', 'A'), conv('b')], 'A')
    expect(result.changed).toBe(true)
    expect(result.conversations.map((c) => c.owner)).toEqual(['A', 'A'])
    expect(result.conversations.map((c) => c.localOnly)).toEqual([
      undefined,
      true,
    ])
  })

  it('never reassigns a conversation owned by another account', () => {
    const result = backfillConversationOwners([conv('a', 'B')], 'A')
    expect(result.conversations[0].owner).toBe('B')
  })

  it('reports no change when every conversation already has an owner', () => {
    const input = [conv('a', 'A'), conv('b', 'B')]
    const result = backfillConversationOwners(input, 'A')
    expect(result.changed).toBe(false)
    expect(result.conversations).toBe(input)
  })

  it('reports no change for an empty list', () => {
    expect(backfillConversationOwners([], 'A').changed).toBe(false)
  })
})

describe('stampConversationOwner', () => {
  it('stamps the effective owner on the matching conversation only', () => {
    const result = stampConversationOwner([conv('a'), conv('b')], 'a', 'A')
    expect(result.map((c) => [c.id, c.owner])).toEqual([
      ['a', 'A'],
      ['b', undefined],
    ])
  })

  it('keeps a signed-out continuation scoped to the last signed-in user', () => {
    const [stamped] = stampConversationOwner([conv('a', 'A')], 'a', 'A')
    expect(filterConversationsByOwner([stamped], 'A').map((c) => c.id)).toEqual(
      ['a'],
    )
  })

  it('stamps undefined for a truly anonymous save', () => {
    const [stamped] = stampConversationOwner([conv('a')], 'a', undefined)
    expect(stamped.owner).toBeUndefined()
  })
})

describe('selectUploadableConversations', () => {
  const backfilled = (id: string, owner: string): Conversation => ({
    ...conv(id, owner),
    localOnly: true,
  })

  it('excludes backfill-adopted (local-only) records from upload', () => {
    const input = [conv('a', 'A'), backfilled('b', 'A'), conv('c', 'A')]
    expect(selectUploadableConversations(input).map((c) => c.id)).toEqual([
      'a',
      'c',
    ])
  })

  it('keeps everything when no record is local-only', () => {
    const input = [conv('a', 'A'), conv('b', 'A')]
    expect(selectUploadableConversations(input)).toEqual(input)
  })
})
