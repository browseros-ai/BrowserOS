import { storage } from '@wxt-dev/storage'
import {
  type ActiveConversationBufferEntry,
  removeBufferEntries,
  selectBufferEntriesForUser,
  upsertBufferEntry,
} from './active-conversation-buffer.helpers'
import type { Conversation } from './conversationStorage'

export type { ActiveConversationBufferEntry } from './active-conversation-buffer.helpers'

/**
 * Transient buffer of in-flight signed-in conversations. Never read by the
 * history panel; it only guarantees the cloud eventually receives the
 * conversation. Entries are removed once flushed, so it never grows into a
 * local mirror of history.
 */
export const activeConversationBufferStorage = storage.defineItem<
  ActiveConversationBufferEntry[]
>('local:activeConversationBuffer', { fallback: [] })

/** Persists (upserts) the in-flight conversation into the buffer. */
export async function bufferActiveConversation(
  entry: ActiveConversationBufferEntry,
): Promise<void> {
  const current = (await activeConversationBufferStorage.getValue()) ?? []
  await activeConversationBufferStorage.setValue(
    upsertBufferEntry(current, entry),
  )
}

/**
 * Uploads the current user's buffered conversations to the cloud via the given
 * idempotent uploader, then removes the flushed entries. Only the current
 * user's entries are touched, so a previous account's un-synced conversation is
 * never pushed into this account's cloud.
 */
export async function flushActiveConversationBuffer(
  userId: string,
  upload: (conversations: Conversation[]) => Promise<void>,
): Promise<void> {
  const current = (await activeConversationBufferStorage.getValue()) ?? []
  const mine = selectBufferEntriesForUser(current, userId)
  if (mine.length === 0) return

  await upload(
    mine.map(({ id, messages, lastMessagedAt }) => ({
      id,
      messages,
      lastMessagedAt,
    })),
  )

  const latest = (await activeConversationBufferStorage.getValue()) ?? []
  await activeConversationBufferStorage.setValue(
    removeBufferEntries(
      latest,
      mine.map((e) => e.id),
    ),
  )
}
