import { describe, expect, it, mock } from 'bun:test'
import type { UIMessage } from 'ai'
import type { Conversation } from '@/lib/conversations/conversationStorage'
import { migrateLegacyConversations } from './conversations-migration.helpers'

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
