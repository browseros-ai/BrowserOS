import { describe, expect, it, mock } from 'bun:test'
import type { UIMessage } from 'ai'
import type { Conversation } from '@/lib/conversations/conversationStorage'
import {
  collectServerConversations,
  createSerialRunner,
  migrateLegacyConversations,
  promoteServerConversations,
} from './conversations-migration.helpers'

function conversation(id: string): Conversation {
  const messages: UIMessage[] = [
    { id: `${id}-m`, role: 'user', parts: [{ type: 'text', text: id }] },
  ]
  return { id, messages, lastMessagedAt: 1 }
}

describe('migrateLegacyConversations', () => {
  it('does nothing when there are no conversations', async () => {
    const importToServer = mock(async () => {})
    const uploadToCloud = mock(async () => [])

    expect(
      await migrateLegacyConversations({
        conversations: [],
        isLoggedIn: false,
        userId: undefined,
        importToServer,
        uploadToCloud,
      }),
    ).toEqual([])
    expect(importToServer).not.toHaveBeenCalled()
    expect(uploadToCloud).not.toHaveBeenCalled()
  })

  it('uploads to the cloud when logged in', async () => {
    const importToServer = mock(async () => {})
    const uploadToCloud = mock(async () => ['a'])

    const handled = await migrateLegacyConversations({
      conversations: [conversation('a'), conversation('b')],
      isLoggedIn: true,
      userId: 'user-1',
      importToServer,
      uploadToCloud,
    })

    expect(handled).toEqual(['a'])
    expect(uploadToCloud).toHaveBeenCalledWith(
      [conversation('a'), conversation('b')],
      'user-1',
    )
    expect(importToServer).not.toHaveBeenCalled()
  })

  it('imports to the server when logged out', async () => {
    const importToServer = mock(async () => {})
    const uploadToCloud = mock(async () => [])

    const handled = await migrateLegacyConversations({
      conversations: [conversation('a'), conversation('b')],
      isLoggedIn: false,
      userId: undefined,
      importToServer,
      uploadToCloud,
    })

    expect(handled).toEqual(['a', 'b'])
    expect(importToServer).toHaveBeenCalledTimes(2)
    expect(uploadToCloud).not.toHaveBeenCalled()
  })

  it('only reports the conversations that imported successfully', async () => {
    const importToServer = mock(async (conv: Conversation) => {
      if (conv.id === 'b') throw new Error('server down')
    })

    const handled = await migrateLegacyConversations({
      conversations: [conversation('a'), conversation('b'), conversation('c')],
      isLoggedIn: false,
      userId: undefined,
      importToServer,
      uploadToCloud: mock(async () => []),
    })

    expect(handled).toEqual(['a', 'c'])
  })

  it('does not touch the server for a logged-in state that lacks a user id', async () => {
    const importToServer = mock(async () => {})
    const uploadToCloud = mock(async () => [])

    expect(
      await migrateLegacyConversations({
        conversations: [conversation('a')],
        isLoggedIn: true,
        userId: undefined,
        importToServer,
        uploadToCloud,
      }),
    ).toEqual([])
    expect(importToServer).not.toHaveBeenCalled()
    expect(uploadToCloud).not.toHaveBeenCalled()
  })
})

describe('collectServerConversations', () => {
  it('pairs each summary with its detail and carries lastMessagedAt', async () => {
    const listSummaries = mock(async () => [
      { id: 'a', lastMessagedAt: 10 },
      { id: 'b', lastMessagedAt: 20 },
    ])
    const loadDetail = mock(async (id: string) => ({
      id,
      messages: [{ id: `${id}-m`, role: 'user', parts: [] }] as UIMessage[],
    }))

    const result = await collectServerConversations({
      listSummaries,
      loadDetail,
    })

    expect(result).toEqual([
      {
        id: 'a',
        lastMessagedAt: 10,
        messages: [{ id: 'a-m', role: 'user', parts: [] }],
      },
      {
        id: 'b',
        lastMessagedAt: 20,
        messages: [{ id: 'b-m', role: 'user', parts: [] }],
      },
    ])
  })

  it('drops a conversation deleted before its detail loaded', async () => {
    const listSummaries = mock(async () => [
      { id: 'a', lastMessagedAt: 10 },
      { id: 'gone', lastMessagedAt: 5 },
    ])
    const loadDetail = mock(async (id: string) =>
      id === 'gone' ? null : { id, messages: [] as UIMessage[] },
    )

    const result = await collectServerConversations({
      listSummaries,
      loadDetail,
    })

    expect(result.map((conversation) => conversation.id)).toEqual(['a'])
  })
})

describe('promoteServerConversations', () => {
  it('does nothing when the server has no conversations', async () => {
    const upload = mock(async () => [])
    const drain = mock(async () => {})

    const result = await promoteServerConversations({
      userId: 'u1',
      collect: mock(async () => []),
      upload,
      drain,
    })

    expect(result).toEqual({ uploadedIds: [], allUploaded: true })
    expect(upload).not.toHaveBeenCalled()
    expect(drain).not.toHaveBeenCalled()
  })

  it('drains every conversation the cloud confirms', async () => {
    const drain = mock((_id: string) => Promise.resolve())

    const result = await promoteServerConversations({
      userId: 'u1',
      collect: mock(async () => [conversation('a'), conversation('b')]),
      upload: mock(async () => ['a', 'b']),
      drain,
    })

    expect(result).toEqual({ uploadedIds: ['a', 'b'], allUploaded: true })
    expect(drain.mock.calls.map((call) => call[0]).sort()).toEqual(['a', 'b'])
  })

  it('keeps a failed conversation on the server and reports incomplete', async () => {
    const drain = mock(async () => {})

    const result = await promoteServerConversations({
      userId: 'u1',
      collect: mock(async () => [conversation('a'), conversation('b')]),
      upload: mock(async () => ['a']),
      drain,
    })

    expect(result).toEqual({ uploadedIds: ['a'], allUploaded: false })
    expect(drain).toHaveBeenCalledTimes(1)
    expect(drain).toHaveBeenCalledWith('a')
  })
})

describe('createSerialRunner', () => {
  it('runs tasks one at a time', async () => {
    const run = createSerialRunner()
    const order: string[] = []

    const first = run(async () => {
      order.push('1-start')
      await Promise.resolve()
      order.push('1-end')
    })
    const second = run(async () => {
      order.push('2-start')
      order.push('2-end')
    })
    await Promise.all([first, second])

    expect(order).toEqual(['1-start', '1-end', '2-start', '2-end'])
  })

  it('runs the next task even when the previous one rejects', async () => {
    const run = createSerialRunner()
    const ran: string[] = []

    const first = run(async () => {
      throw new Error('boom')
    })
    const second = run(async () => {
      ran.push('second')
    })

    await expect(first).rejects.toThrow('boom')
    await second
    expect(ran).toEqual(['second'])
  })
})
