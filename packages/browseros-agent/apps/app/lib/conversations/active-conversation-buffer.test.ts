import { describe, expect, it } from 'bun:test'
import {
  type ActiveConversationBufferEntry,
  removeBufferEntries,
  selectBufferEntriesForUser,
  upsertBufferEntry,
} from './active-conversation-buffer.helpers'

const entry = (id: string, userId: string): ActiveConversationBufferEntry => ({
  id,
  userId,
  messages: [],
  lastMessagedAt: 0,
})

describe('upsertBufferEntry', () => {
  it('appends a conversation that is not buffered yet', () => {
    const result = upsertBufferEntry([entry('a', 'A')], entry('b', 'A'))
    expect(result.map((e) => e.id)).toEqual(['a', 'b'])
  })

  it('replaces an existing conversation instead of duplicating it', () => {
    const updated = { ...entry('a', 'A'), lastMessagedAt: 5 }
    const result = upsertBufferEntry(
      [entry('a', 'A'), entry('b', 'A')],
      updated,
    )
    expect(result.map((e) => e.id)).toEqual(['b', 'a'])
    expect(result.find((e) => e.id === 'a')?.lastMessagedAt).toBe(5)
  })
})

describe('selectBufferEntriesForUser', () => {
  it('keeps only the given user and never another account', () => {
    const all = [entry('a', 'A'), entry('b', 'B'), entry('c', 'A')]
    expect(selectBufferEntriesForUser(all, 'A').map((e) => e.id)).toEqual([
      'a',
      'c',
    ])
    expect(selectBufferEntriesForUser(all, 'B').map((e) => e.id)).toEqual(['b'])
  })
})

describe('removeBufferEntries', () => {
  it('drops the flushed ids and keeps the rest', () => {
    const all = [entry('a', 'A'), entry('b', 'A'), entry('c', 'A')]
    expect(removeBufferEntries(all, ['a', 'c']).map((e) => e.id)).toEqual(['b'])
  })

  it('returns the remaining entries when nothing matches', () => {
    expect(
      removeBufferEntries([entry('a', 'A')], ['x']).map((e) => e.id),
    ).toEqual(['a'])
  })
})
