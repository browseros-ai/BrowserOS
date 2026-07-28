import { describe, expect, it } from 'bun:test'
import {
  type ActiveConversationBufferEntry,
  pruneFlushedEntries,
  runExclusiveBufferWrite,
  selectBufferEntriesForUser,
  upsertBufferEntry,
} from './active-conversation-buffer.helpers'

const entry = (
  id: string,
  userId: string,
  lastMessagedAt = 0,
): ActiveConversationBufferEntry => ({
  id,
  userId,
  messages: [],
  lastMessagedAt,
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

describe('pruneFlushedEntries', () => {
  it('removes only the exact snapshots that were flushed', () => {
    const all = [entry('a', 'A', 1), entry('b', 'A', 1), entry('c', 'A', 1)]
    const flushed = [entry('a', 'A', 1), entry('c', 'A', 1)]
    expect(pruneFlushedEntries(all, flushed).map((e) => e.id)).toEqual(['b'])
  })

  it('keeps a newer snapshot written for the same id during the upload', () => {
    // 'a' was uploaded at t=1 but rewritten at t=2 while the upload ran.
    const latest = [entry('a', 'A', 2), entry('b', 'A', 1)]
    const flushed = [entry('a', 'A', 1)]
    expect(pruneFlushedEntries(latest, flushed).map((e) => e.id)).toEqual([
      'a',
      'b',
    ])
  })

  it('keeps entries whose upload was not confirmed', () => {
    const latest = [entry('a', 'A', 1), entry('b', 'A', 1)]
    expect(pruneFlushedEntries(latest, []).map((e) => e.id)).toEqual(['a', 'b'])
  })
})

describe('runExclusiveBufferWrite', () => {
  it('serializes overlapping mutations instead of interleaving them', async () => {
    const order: string[] = []
    const first = runExclusiveBufferWrite(async () => {
      order.push('a-start')
      await Promise.resolve()
      await Promise.resolve()
      order.push('a-end')
    })
    const second = runExclusiveBufferWrite(async () => {
      order.push('b-start')
      order.push('b-end')
    })
    await Promise.all([first, second])
    expect(order).toEqual(['a-start', 'a-end', 'b-start', 'b-end'])
  })

  it('keeps draining the queue after a mutation rejects', async () => {
    await expect(
      runExclusiveBufferWrite(async () => {
        throw new Error('boom')
      }),
    ).rejects.toThrow('boom')
    await expect(runExclusiveBufferWrite(async () => 'ok')).resolves.toBe('ok')
  })
})
