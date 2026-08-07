import { describe, expect, it } from 'bun:test'
import { createConversationUploadScheduler } from './conversation-upload-scheduler'
import type { Conversation } from './conversationStorage'

describe('createConversationUploadScheduler', () => {
  it('uploads only the newest snapshot queued during the debounce window', async () => {
    const uploads: string[][] = []
    const schedule = createConversationUploadScheduler(
      async (conversations) => {
        uploads.push(conversations.map((conversation) => conversation.id))
      },
      { delayMs: 5 },
    )

    schedule([conversation('older')])
    schedule([conversation('newest')])
    await Bun.sleep(20)

    expect(uploads).toEqual([['newest']])
  })

  it('cancels a pending snapshot when the current list becomes empty', async () => {
    const uploads: string[][] = []
    const schedule = createConversationUploadScheduler(
      async (conversations) => {
        uploads.push(conversations.map((conversation) => conversation.id))
      },
      { delayMs: 5 },
    )

    schedule([conversation('deleted-before-upload')])
    schedule([])
    await Bun.sleep(20)

    expect(uploads).toEqual([])
  })

  it('keeps pending work when another caller has not hydrated yet', async () => {
    const uploads: string[][] = []
    const schedule = createConversationUploadScheduler(
      async (conversations) => {
        uploads.push(conversations.map((conversation) => conversation.id))
      },
      { delayMs: 5 },
    )

    schedule([conversation('hydrated')])
    schedule(undefined)
    await Bun.sleep(20)

    expect(uploads).toEqual([['hydrated']])
  })

  it('waits for an active upload and keeps only the newest pending snapshot', async () => {
    const uploads: string[][] = []
    const firstUploadStarted = Promise.withResolvers<void>()
    const releaseFirstUpload = Promise.withResolvers<void>()
    const schedule = createConversationUploadScheduler(
      async (conversations) => {
        uploads.push(conversations.map((conversation) => conversation.id))
        if (uploads.length === 1) {
          firstUploadStarted.resolve()
          await releaseFirstUpload.promise
        }
      },
      { delayMs: 5 },
    )

    schedule([conversation('first')])
    await firstUploadStarted.promise
    schedule([conversation('superseded')])
    schedule([conversation('latest')])
    await Bun.sleep(20)

    expect(uploads).toEqual([['first']])

    releaseFirstUpload.resolve()
    await Bun.sleep(20)

    expect(uploads).toEqual([['first'], ['latest']])
  })

  it('does not queue the active snapshot a second time', async () => {
    const uploads: string[][] = []
    const uploadStarted = Promise.withResolvers<void>()
    const releaseUpload = Promise.withResolvers<void>()
    const schedule = createConversationUploadScheduler(
      async (conversations) => {
        uploads.push(conversations.map((conversation) => conversation.id))
        uploadStarted.resolve()
        await releaseUpload.promise
      },
      { delayMs: 5 },
    )

    schedule([conversation('active')])
    await uploadStarted.promise
    schedule([conversation('active')])
    releaseUpload.resolve()
    await Bun.sleep(20)

    expect(uploads).toEqual([['active']])
  })

  it('deduplicates completed snapshots within one account', async () => {
    const uploads: string[][] = []
    const schedule = createConversationUploadScheduler(
      async (conversations) => {
        uploads.push(conversations.map((conversation) => conversation.id))
      },
      { delayMs: 5 },
    )

    schedule([conversation('local')], 'user-a')
    await Bun.sleep(20)
    schedule([conversation('local')], 'user-a')
    await Bun.sleep(20)
    schedule([conversation('local')], 'user-b')
    await Bun.sleep(20)

    expect(uploads).toEqual([['local'], ['local']])
  })
})

function conversation(id: string): Conversation {
  return { id, messages: [], lastMessagedAt: 0 }
}
