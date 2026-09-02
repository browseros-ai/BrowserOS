import { describe, expect, it } from 'bun:test'
import type { Conversation } from '@/lib/conversations/conversationStorage'
import {
  createSerialRunner,
  migrateLegacyConversations,
} from './conversations-migration.helpers'

function conversation(id: string, messageIds: string[] = ['m1']): Conversation {
  return {
    id,
    messages: messageIds.map((mid) => ({ id: mid })),
    lastMessagedAt: 1,
  } as unknown as Conversation
}

const neverLoaded = async () => {
  throw new Error('should not read the server row')
}

describe('migrateLegacyConversations', () => {
  it('does nothing when there is nothing to migrate', async () => {
    const handled = await migrateLegacyConversations({
      conversations: [],
      importToServer: async () => {
        throw new Error('should not be called')
      },
      loadFromServer: neverLoaded,
    })
    expect(handled).toEqual([])
  })

  it('imports every conversation into the local server', async () => {
    const imported: string[] = []
    const handled = await migrateLegacyConversations({
      conversations: [conversation('a'), conversation('b')],
      importToServer: async (c) => {
        imported.push(c.id)
        return { imported: true }
      },
      loadFromServer: neverLoaded,
    })
    expect(imported).toEqual(['a', 'b'])
    expect(handled).toEqual(['a', 'b'])
  })

  // A conversation that fails is left in storage so the next attempt retries
  // it, rather than being dropped as handled.
  it('reports only the conversations that imported', async () => {
    const handled = await migrateLegacyConversations({
      conversations: [conversation('a'), conversation('b'), conversation('c')],
      importToServer: async (c) => {
        if (c.id === 'b') throw new Error('server unavailable')
        return { imported: true }
      },
      loadFromServer: neverLoaded,
    })
    expect(handled).toEqual(['a', 'c'])
  })

  it('reports nothing when the server is unreachable', async () => {
    const handled = await migrateLegacyConversations({
      conversations: [conversation('a')],
      importToServer: async () => {
        throw new Error('server unavailable')
      },
      loadFromServer: neverLoaded,
    })
    expect(handled).toEqual([])
  })
})

// The import is insert-if-absent: an id already on the server answers with a
// success that wrote nothing. Draining on that alone deletes the legacy copy
// against a row that may be an older, shorter version of it.
describe('migrateLegacyConversations when the import is skipped', () => {
  const skipped = async () => ({ imported: false })

  it('drains when the server row already holds every message', async () => {
    const handled = await migrateLegacyConversations({
      conversations: [conversation('a', ['m1', 'm2'])],
      importToServer: skipped,
      loadFromServer: async () => ({
        messages: [{ id: 'm1' }, { id: 'm2' }, { id: 'm3' }],
      }),
    })
    expect(handled).toEqual(['a'])
  })

  it('keeps the legacy copy when the server row is missing messages', async () => {
    const handled = await migrateLegacyConversations({
      conversations: [conversation('a', ['m1', 'm2'])],
      importToServer: skipped,
      loadFromServer: async () => ({ messages: [{ id: 'm1' }] }),
    })
    expect(handled).toEqual([])
  })

  // Same length, different messages: a count comparison would wrongly drain.
  it('keeps the legacy copy when the server row differs but is the same size', async () => {
    const handled = await migrateLegacyConversations({
      conversations: [conversation('a', ['m1', 'm2'])],
      importToServer: skipped,
      loadFromServer: async () => ({
        messages: [{ id: 'm1' }, { id: 'other' }],
      }),
    })
    expect(handled).toEqual([])
  })

  it('keeps the legacy copy when the server row cannot be read', async () => {
    const handled = await migrateLegacyConversations({
      conversations: [conversation('a')],
      importToServer: skipped,
      loadFromServer: async () => null,
    })
    expect(handled).toEqual([])
  })

  it('does not drain when reading the server row throws', async () => {
    const handled = await migrateLegacyConversations({
      conversations: [conversation('a')],
      importToServer: skipped,
      loadFromServer: async () => {
        throw new Error('server unavailable')
      },
    })
    expect(handled).toEqual([])
  })
})

describe('createSerialRunner', () => {
  it('runs tasks one at a time in order', async () => {
    const run = createSerialRunner()
    const order: string[] = []
    const task = (id: string, ms: number) => async () => {
      await new Promise((resolve) => setTimeout(resolve, ms))
      order.push(id)
      return id
    }

    await Promise.all([run(task('slow', 20)), run(task('fast', 1))])

    expect(order).toEqual(['slow', 'fast'])
  })

  it('keeps running after a task rejects', async () => {
    const run = createSerialRunner()
    await run(async () => {
      throw new Error('boom')
    }).catch(() => undefined)

    await expect(run(async () => 'next')).resolves.toBe('next')
  })
})
