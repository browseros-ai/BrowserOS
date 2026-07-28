import { storage } from '@wxt-dev/storage'
import {
  type ActiveConversationBufferEntry,
  pruneFlushedEntries,
  runExclusiveBufferWrite,
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
  await runExclusiveBufferWrite(async () => {
    const current = (await activeConversationBufferStorage.getValue()) ?? []
    await activeConversationBufferStorage.setValue(
      upsertBufferEntry(current, entry),
    )
  })
}

/**
 * Uploads the current user's buffered conversations to the cloud via the given
 * uploader (which returns the ids it confirmed reached the cloud), then removes
 * only those exact snapshots. Entries whose upload failed, and any newer
 * snapshot written for the same conversation during the upload, are kept for a
 * later retry. Only the current user's entries are ever touched, so a previous
 * account's un-synced conversation is never pushed into this account's cloud.
 */
export async function flushActiveConversationBuffer(
  userId: string,
  upload: (conversations: Conversation[]) => Promise<string[]>,
): Promise<void> {
  const current = (await activeConversationBufferStorage.getValue()) ?? []
  const mine = selectBufferEntriesForUser(current, userId)
  if (mine.length === 0) return

  const uploadedIds = new Set(
    await upload(
      mine.map(({ id, messages, lastMessagedAt }) => ({
        id,
        messages,
        lastMessagedAt,
      })),
    ),
  )
  const flushed = mine.filter((e) => uploadedIds.has(e.id))
  if (flushed.length === 0) return

  // The upload above ran outside the lock so writes weren't blocked; take the
  // lock only to re-read the latest buffer and remove the flushed snapshots, so
  // a concurrent write is neither clobbered by nor clobbers this prune.
  await runExclusiveBufferWrite(async () => {
    const latest = (await activeConversationBufferStorage.getValue()) ?? []
    await activeConversationBufferStorage.setValue(
      pruneFlushedEntries(latest, flushed),
    )
  })
}
