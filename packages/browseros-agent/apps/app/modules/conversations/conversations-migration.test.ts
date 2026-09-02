import { describe, expect, it } from 'bun:test'
import type { Conversation } from '@/lib/conversations/conversationStorage'
import {
  createSerialRunner,
  migrateLegacyConversations,
} from './conversations-migration.helpers'

function conversation(id: string): Conversation {
  return { id, messages: [], lastMessagedAt: 1 } as unknown as Conversation
}

describe('migrateLegacyConversations', () => {
  it('does nothing when there is nothing to migrate', async () => {
    const handled = await migrateLegacyConversations({
      conversations: [],
      importToServer: async () => {
        throw new Error('should not be called')
      },
    })
    expect(handled).toEqual([])
  })

  it('imports every conversation into the local server', async () => {
    const imported: string[] = []
    const handled = await migrateLegacyConversations({
      conversations: [conversation('a'), conversation('b')],
      importToServer: async (c) => {
        imported.push(c.id)
      },
    })
    expect(imported).toEqual(['a', 'b'])
    expect(handled).toEqual(['a', 'b'])
  })

  // A signed-in user used to have their history sent to the cloud instead.
  // Everyone takes the local path now, which is the direction this work moves
  // data, and the caller no longer passes identity at all.
  it('takes the local path regardless of any session', async () => {
    const imported: string[] = []
    await migrateLegacyConversations({
      conversations: [conversation('a')],
      importToServer: async (c) => {
        imported.push(c.id)
      },
    })
    expect(imported).toEqual(['a'])
  })

  // A conversation that fails is left in storage so the next attempt retries
  // it, rather than being dropped as handled.
  it('reports only the conversations that imported', async () => {
    const handled = await migrateLegacyConversations({
      conversations: [conversation('a'), conversation('b'), conversation('c')],
      importToServer: async (c) => {
        if (c.id === 'b') throw new Error('server unavailable')
      },
    })
    expect(handled).toEqual(['a', 'c'])
  })

  it('reports nothing when the server is unreachable', async () => {
    const handled = await migrateLegacyConversations({
      conversations: [conversation('a')],
      importToServer: async () => {
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
