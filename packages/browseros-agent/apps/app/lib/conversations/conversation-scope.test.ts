import { describe, expect, it } from 'bun:test'
import {
  filterConversationsByOwner,
  resolveEffectiveOwnerId,
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
